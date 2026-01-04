use crate::ringbuffer::{RRProfTraceEvent, RRProfTraceEventType};
use gpu_sync_vec::GpuSyncVec;
use std::cmp::Reverse;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::fmt::{Debug, Formatter};
use std::{fmt, iter, mem};
use wgpu::{Buffer, BufferUsages, Device, Queue};

mod gpu_sync_vec;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CallBox {
    start_time: [u32; 2],
    end_time: [u32; 2],
    method_id: u32,
    depth: u32,
}

fn encode_time(time: u64) -> [u32; 2] {
    [
        (time & 0x7fffffff) as u32,
        ((time >> 31) & 0xffffffff) as u32,
    ]
}

struct CallStackEntry {
    vertex_index: usize,
    method_id: u64,
}

struct ThreadStack {
    call_stack: Vec<CallStackEntry>,
    vertices: GpuSyncVec<CallBox>,
    ready_for_free_slot: VecDeque<(usize, u64)>,
    free_slot: BinaryHeap<Reverse<usize>>,
    used_slot: BTreeSet<usize>,
    visible_call_depth: MultiSet<u32>,
    free_depth: VecDeque<(u32, u64)>,
}

impl ThreadStack {
    fn new(device: Device, queue: Queue) -> ThreadStack {
        const VERTEX_BUFFER_USAGE: BufferUsages =
            BufferUsages::VERTEX.union(BufferUsages::COPY_DST);
        ThreadStack {
            call_stack: Vec::new(),
            vertices: GpuSyncVec::new(device, queue, VERTEX_BUFFER_USAGE),
            ready_for_free_slot: VecDeque::new(),
            free_slot: BinaryHeap::new(),
            used_slot: Default::default(),
            visible_call_depth: MultiSet::new(),
            free_depth: VecDeque::new(),
        }
    }

    fn sync_free_slot(&mut self, at: u64) {
        while let Some(&(index, exit_at)) = self.ready_for_free_slot.front()
            && exit_at + VISIBLE_DURATION < at
        {
            self.ready_for_free_slot.pop_front();
            self.free_slot.push(Reverse(index));
            self.used_slot.remove(&index);
        }
    }

    fn enter(&mut self, at: u64, enter_method_id: u64) {
        self.sync_free_slot(at);
        let depth = self.call_stack.len() as u32;
        let vertex = CallBox {
            start_time: encode_time(at),
            end_time: encode_time(u64::MAX),
            depth,
            method_id: enter_method_id as u32,
        };
        let index = if let Some(Reverse(index)) = self.free_slot.pop() {
            self.vertices[index] = vertex;
            index
        } else {
            let index = self.vertices.len();
            self.vertices.push(vertex);
            index
        };
        self.call_stack.push(CallStackEntry {
            vertex_index: index,
            method_id: enter_method_id,
        });
        self.used_slot.insert(index);
        self.visible_call_depth.insert(depth);
    }

    fn exit(&mut self, at: u64, exit_method_id: u64) {
        while let Some(entry) = self.call_stack.pop() {
            let CallStackEntry {
                vertex_index,
                method_id,
            } = entry;
            let depth = self.call_stack.len() as u32;
            self.free_depth.push_back((depth, at));
            if vertex_index != usize::MAX {
                self.vertices[vertex_index].end_time = encode_time(at);
                self.ready_for_free_slot.push_back((vertex_index, at));
            }
            if exit_method_id == method_id {
                break;
            }
        }
    }

    fn cut_all(&mut self, at: u64) {
        for depth in 0..self.call_stack.len() as u32 {
            self.free_depth.push_back((depth, at));
        }
        for CallStackEntry { vertex_index, .. } in self.call_stack.iter_mut() {
            let index = mem::replace(vertex_index, usize::MAX);
            self.vertices[index].end_time = encode_time(at);
            self.ready_for_free_slot.push_back((index, at));
        }
    }

    fn resume_all(&mut self, at: u64) {
        self.sync_free_slot(at);
        for depth in 0..self.call_stack.len() as u32 {
            self.visible_call_depth.insert(depth);
        }
        for (
            depth,
            &mut CallStackEntry {
                ref mut vertex_index,
                method_id,
            },
        ) in self.call_stack.iter_mut().enumerate()
        {
            let vertex = CallBox {
                start_time: encode_time(at),
                end_time: encode_time(u64::MAX),
                method_id: method_id as u32,
                depth: depth as u32,
            };
            let index = if let Some(Reverse(index)) = self.free_slot.pop() {
                self.vertices[index] = vertex;
                index
            } else {
                let index = self.vertices.len();
                self.vertices.push(vertex);
                index
            };
            *vertex_index = index;
            self.used_slot.insert(index);
        }
    }

    fn sync(&mut self, now: u64) {
        let required_len = self.used_slot.last().map_or(0, |&last| last + 1);
        self.vertices.truncate(required_len);
        self.vertices.sync();
        while let Some(&(depth, exit_at)) = self.free_depth.front()
            && exit_at + VISIBLE_DURATION < now
        {
            self.free_depth.pop_front();
            self.visible_call_depth.remove(depth);
        }
    }
}

pub struct TraceState {
    device: Device,
    queue: Queue,
    thread_stacks: BTreeMap<u32, ThreadStack>,
    base_time: u64,
    last_thread_id: u32,
    exited_threads: VecDeque<(u32, u64)>,
}

const VISIBLE_DURATION: u64 = 1_000_000_000 * 30;

impl TraceState {
    pub fn new(device: Device, queue: Queue) -> TraceState {
        TraceState {
            device,
            queue,
            thread_stacks: BTreeMap::new(),
            base_time: 0,
            last_thread_id: 0,
            exited_threads: VecDeque::new(),
        }
    }

    pub fn base_time(&self) -> [u32; 2] {
        encode_time(self.base_time)
    }

    pub fn num_threads(&self) -> u32 {
        self.thread_stacks.len() as u32
    }

    pub fn process_events(&mut self, events: &[RRProfTraceEvent]) {
        macro_rules! thread_data {
            ($tid:expr) => {
                self.thread_stacks
                    .entry($tid)
                    .or_insert_with(|| ThreadStack::new(self.device.clone(), self.queue.clone()))
            };
        }
        for event in events {
            let timestamp = event.timestamp();
            let event_type = event.event_type();
            self.base_time = timestamp;

            match event_type {
                RRProfTraceEventType::Call => {
                    let tid = self.last_thread_id;
                    let stack: &mut ThreadStack = thread_data!(tid);
                    stack.enter(timestamp, event.data());
                }
                RRProfTraceEventType::Return => {
                    let tid = self.last_thread_id;
                    let stack: &mut ThreadStack = thread_data!(tid);
                    stack.exit(timestamp, event.data());
                }
                RRProfTraceEventType::GCStart => {
                    let tid = self.last_thread_id;
                    let stack: &mut ThreadStack = thread_data!(tid);
                    stack.cut_all(timestamp);
                }
                RRProfTraceEventType::GCEnd => {
                    let tid = self.last_thread_id;
                    let stack: &mut ThreadStack = thread_data!(tid);
                    stack.resume_all(timestamp);
                }
                RRProfTraceEventType::ThreadSuspended => {
                    let tid = event.data() as u32;
                    let stack: &mut ThreadStack = thread_data!(tid);
                    stack.cut_all(timestamp);
                }
                RRProfTraceEventType::ThreadResume => {
                    let tid = event.data() as u32;
                    self.last_thread_id = tid;
                    let stack: &mut ThreadStack = thread_data!(tid);
                    stack.resume_all(timestamp);
                }
                RRProfTraceEventType::ThreadExit => {
                    let tid = event.data() as u32;
                    self.exited_threads.push_back((tid, timestamp));
                }
                RRProfTraceEventType::ThreadStart | RRProfTraceEventType::ThreadReady => {}
            }
        }
    }

    pub fn max_depth(&mut self) -> u32 {
        self.thread_stacks
            .values_mut()
            .filter_map(|stack| stack.visible_call_depth.max().copied())
            .max()
            .unwrap_or(0)
    }

    pub fn sync(&mut self) {
        while let Some(&(tid, exited_at)) = self.exited_threads.front() {
            if self.base_time <= exited_at + VISIBLE_DURATION {
                break;
            }
            self.exited_threads.pop_front();
            self.thread_stacks.remove(&tid);
        }
        self.thread_stacks
            .values_mut()
            .for_each(|stack| stack.sync(self.base_time));
    }

    pub fn read_vertices(&mut self, mut f: impl FnMut(usize, &Buffer, usize)) {
        for (i, stack) in self.thread_stacks.values_mut().enumerate() {
            stack.sync(self.base_time);
            f(i, stack.vertices.buffer(), stack.vertices.len());
        }
    }
}

struct MultiSet<T> {
    inner: BTreeMap<T, usize>,
}

impl<T> Default for MultiSet<T> {
    fn default() -> Self {
        MultiSet::new()
    }
}

impl<T> Debug for MultiSet<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.inner.iter().flat_map(|(k, &v)| iter::repeat_n(k, v)))
            .finish()
    }
}

impl<T> MultiSet<T> {
    fn new() -> MultiSet<T> {
        MultiSet {
            inner: BTreeMap::new(),
        }
    }
}
impl<T> MultiSet<T>
where
    T: Ord,
{
    fn insert(&mut self, value: T) {
        *self.inner.entry(value).or_default() += 1;
    }

    fn remove(&mut self, value: T) {
        match self.inner.entry(value) {
            Entry::Vacant(_) => return,
            Entry::Occupied(mut entry) => {
                let count = entry.get_mut();
                if *count <= 1 {
                    entry.remove();
                } else {
                    *count -= 1;
                }
            }
        }
    }

    fn max(&self) -> Option<&T> {
        self.inner.last_key_value().map(|(v, _)| v)
    }
}
