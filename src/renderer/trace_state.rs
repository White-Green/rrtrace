use crate::ringbuffer::{RRProfTraceEvent, RRProfTraceEventType};
use gpu_sync_vec::GpuSyncVec;
use std::collections::{BTreeMap, VecDeque};
use std::mem;
use wgpu::{Buffer, BufferUsages, Device, Queue};

mod gpu_sync_vec;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuBox {
    start_time: u64,
    end_time: u64,
    method_id: u64,
    depth: u64,
}

struct CallStackEntry {
    vertex_index: usize,
    method_id: u64,
}

struct ThreadStack {
    call_stack: Vec<CallStackEntry>,
    vertices: GpuSyncVec<GpuBox>,
    free_slot: VecDeque<usize>,
}

impl ThreadStack {
    fn new(device: Device, queue: Queue) -> ThreadStack {
        const VERTEX_BUFFER_USAGE: BufferUsages =
            BufferUsages::VERTEX.union(BufferUsages::COPY_DST);
        ThreadStack {
            call_stack: Vec::new(),
            vertices: GpuSyncVec::new(device, queue, VERTEX_BUFFER_USAGE),
            free_slot: VecDeque::new(),
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
                    let vertex = GpuBox {
                        start_time: timestamp,
                        end_time: u64::MAX,
                        depth: stack.call_stack.len() as u64,
                        method_id: event.data(),
                    };
                    let index = if let Some(&index) = stack.free_slot.front()
                        && stack.vertices[index].end_time < VISIBLE_DURATION
                    {
                        stack.free_slot.pop_front();
                        stack.vertices[index] = vertex;
                        index
                    } else {
                        let index = stack.vertices.len();
                        stack.vertices.push(vertex);
                        index
                    };
                    stack.call_stack.push(CallStackEntry {
                        vertex_index: index,
                        method_id: event.data(),
                    });
                }
                RRProfTraceEventType::Return => {
                    let tid = self.last_thread_id;
                    let event_method_id = event.data();
                    let stack: &mut ThreadStack = thread_data!(tid);
                    while let Some(entry) = stack.call_stack.pop() {
                        let CallStackEntry {
                            vertex_index,
                            method_id,
                        } = entry;
                        stack.vertices[vertex_index].end_time = timestamp;
                        stack.free_slot.push_back(vertex_index);
                        if event_method_id == method_id {
                            break;
                        }
                    }
                }
                RRProfTraceEventType::GCStart => {
                    let tid = self.last_thread_id;
                    let stack: &mut ThreadStack = thread_data!(tid);
                    for CallStackEntry { vertex_index, .. } in stack.call_stack.iter_mut() {
                        let index = mem::replace(vertex_index, usize::MAX);
                        stack.vertices[index].end_time = timestamp;
                        stack.free_slot.push_back(index);
                    }
                }
                RRProfTraceEventType::GCEnd => {
                    let tid = self.last_thread_id;
                    let stack: &mut ThreadStack = thread_data!(tid);
                    for (
                        depth,
                        &mut CallStackEntry {
                            ref mut vertex_index,
                            method_id,
                        },
                    ) in stack.call_stack.iter_mut().enumerate()
                    {
                        let vertex = GpuBox {
                            start_time: timestamp,
                            end_time: u64::MAX,
                            method_id,
                            depth: depth as u64,
                        };
                        let index = if let Some(&index) = stack.free_slot.front()
                            && stack.vertices[index].end_time < VISIBLE_DURATION
                        {
                            stack.free_slot.pop_front();
                            stack.vertices[index] = vertex;
                            index
                        } else {
                            let index = stack.vertices.len();
                            stack.vertices.push(vertex);
                            index
                        };
                        *vertex_index = index;
                    }
                }
                RRProfTraceEventType::ThreadSuspended => {
                    let tid = event.data() as u32;
                    let stack: &mut ThreadStack = thread_data!(tid);
                    for CallStackEntry { vertex_index, .. } in stack.call_stack.iter_mut() {
                        let index = mem::replace(vertex_index, usize::MAX);
                        stack.vertices[index].end_time = timestamp;
                        stack.free_slot.push_back(index);
                    }
                }
                RRProfTraceEventType::ThreadResume => {
                    let tid = event.data() as u32;
                    self.last_thread_id = tid;
                    let stack: &mut ThreadStack = thread_data!(tid);
                    for (
                        depth,
                        &mut CallStackEntry {
                            ref mut vertex_index,
                            method_id,
                        },
                    ) in stack.call_stack.iter_mut().enumerate()
                    {
                        let vertex = GpuBox {
                            start_time: timestamp,
                            end_time: u64::MAX,
                            method_id,
                            depth: depth as u64,
                        };
                        let index = if let Some(&index) = stack.free_slot.front()
                            && stack.vertices[index].end_time < VISIBLE_DURATION
                        {
                            stack.free_slot.pop_front();
                            stack.vertices[index] = vertex;
                            index
                        } else {
                            let index = stack.vertices.len();
                            stack.vertices.push(vertex);
                            index
                        };
                        *vertex_index = index;
                    }
                }
                RRProfTraceEventType::ThreadExit => {
                    let tid = event.data() as u32;
                    self.exited_threads.push_back((tid, timestamp));
                }
                RRProfTraceEventType::ThreadStart | RRProfTraceEventType::ThreadReady => {}
            }
        }
    }

    pub fn read_vertices(&mut self, mut f: impl FnMut(usize, &Buffer, usize)) {
        while let Some(&(tid, exited_at)) = self.exited_threads.front() {
            if exited_at + VISIBLE_DURATION >= self.base_time {
                break;
            }
            self.exited_threads.pop_front();
            self.thread_stacks.remove(&tid);
        }
        for (i, stack) in self.thread_stacks.values_mut().enumerate() {
            stack.vertices.sync();
            f(i, stack.vertices.buffer(), stack.vertices.len());
        }
    }
}
