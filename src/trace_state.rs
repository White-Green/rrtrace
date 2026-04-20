use crate::ringbuffer::{RRTraceEvent, RRTraceEventType};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::Debug;
use std::{convert, iter, mem};

#[repr(C)]
#[derive(Copy, Default, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CallBox {
    start_time: [u32; 2],
    end_time: [u32; 2],
    method_id: u32,
    depth: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadLine {
    start_time: [u32; 2],
    end_time: [u32; 2],
}

impl ThreadLine {
    pub fn start_time(&self) -> [u32; 2] {
        self.start_time
    }

    pub fn end_time(&self) -> [u32; 2] {
        self.end_time
    }
}

#[derive(Debug, Clone)]
pub struct ThreadData {
    thread_id: u32,
    call_boxes: Vec<CallBox>,
    thread_line: ThreadLine,
}

impl ThreadData {
    pub fn thread_id(&self) -> u32 {
        self.thread_id
    }

    pub fn call_boxes(&self) -> &[CallBox] {
        &self.call_boxes
    }

    pub fn thread_line(&self) -> ThreadLine {
        self.thread_line
    }
}

pub const VISIBLE_DURATION: u64 = 1_000_000_000 * 5;

pub fn encode_time(time: u64) -> [u32; 2] {
    [
        (time & 0x7fffffff) as u32,
        ((time >> 31) & 0xffffffff) as u32,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThreadId {
    None,
    Initial,
    Id(u32),
}

#[derive(Debug, Clone, Default)]
struct StackState {
    unmarked_returns: SmallVec<[u64; 2]>,
    stack: SmallVec<[u64; 16]>,
    exited: bool,
}

impl StackState {
    fn new() -> StackState {
        StackState::default()
    }

    #[inline(always)]
    fn call(&mut self, method_id: u64) {
        self.stack.push(method_id);
    }

    #[inline(always)]
    fn ret(&mut self, method_id: u64) {
        loop {
            match self.stack.pop() {
                None => {
                    self.unmarked_returns.push(method_id);
                    break;
                }
                Some(m) if m == method_id => break,
                Some(_) => {}
            }
        }
    }

    #[inline(always)]
    fn exit(&mut self) {
        self.exited = true;
    }

    fn drop_unmarked_returns(&mut self) {
        self.unmarked_returns.clear();
    }

    fn merge_into(&self, other: &mut Self) {
        let additional_push_stack = mem::replace(&mut other.stack, self.stack.clone());
        let unmarked_returns =
            mem::replace(&mut other.unmarked_returns, self.unmarked_returns.clone());
        for method_id in unmarked_returns {
            other.ret(method_id);
        }
        for method_id in additional_push_stack {
            other.call(method_id);
        }
    }
}

#[derive(Debug, Clone)]
pub struct FastTrace {
    thread_stacks: HashMap<u32, StackState>,
    initial_thread_stack: StackState,
    current_thread: ThreadId,
    in_gc: bool,
}

impl Default for FastTrace {
    fn default() -> Self {
        FastTrace {
            thread_stacks: HashMap::new(),
            initial_thread_stack: StackState::new(),
            current_thread: ThreadId::Id(0),
            in_gc: false,
        }
    }
}

impl FastTrace {
    pub fn from_events(events: &[RRTraceEvent]) -> FastTrace {
        let mut thread_stacks = HashMap::<u32, StackState>::new();
        let mut initial_thread_stack = StackState::new();
        let mut current_thread = ThreadId::Initial;

        let mut current_thread_stack = &mut initial_thread_stack;
        for &event in events {
            match event.event_type() {
                RRTraceEventType::Call => {
                    let method_id = event.data();
                    current_thread_stack.call(method_id);
                }
                RRTraceEventType::Return => {
                    let method_id = event.data();
                    current_thread_stack.ret(method_id);
                }
                RRTraceEventType::ThreadSuspended => {
                    current_thread = ThreadId::None;
                }
                RRTraceEventType::ThreadResume => {
                    let thread_id = event.data() as u32;
                    current_thread = ThreadId::Id(thread_id);
                    current_thread_stack = thread_stacks.entry(thread_id).or_default();
                }
                RRTraceEventType::ThreadStart => {
                    let thread_id = event.data() as u32;
                    thread_stacks.insert(thread_id, StackState::new());
                    current_thread_stack = if let ThreadId::Id(current_thread_id) = current_thread {
                        thread_stacks.entry(current_thread_id).or_default()
                    } else {
                        &mut initial_thread_stack
                    };
                }
                RRTraceEventType::ThreadExit => {
                    current_thread_stack.exit();
                }
                RRTraceEventType::GCStart
                | RRTraceEventType::GCEnd
                | RRTraceEventType::ThreadReady => {}
            }
        }
        let in_gc = events.last().unwrap().event_type() == RRTraceEventType::GCStart;
        FastTrace {
            thread_stacks,
            initial_thread_stack,
            current_thread,
            in_gc,
        }
    }

    pub fn mark_as_first(&mut self) {
        self.thread_stacks
            .values_mut()
            .chain(iter::once(&mut self.initial_thread_stack))
            .for_each(StackState::drop_unmarked_returns);
        if let ThreadId::Initial = self.current_thread {
            self.current_thread = ThreadId::Id(0);
        }
        let initial_thread_stack = mem::take(&mut self.initial_thread_stack);
        match self.thread_stacks.entry(0) {
            Entry::Occupied(mut entry) => {
                initial_thread_stack.merge_into(entry.get_mut());
            }
            Entry::Vacant(entry) => {
                entry.insert(initial_thread_stack);
            }
        }
    }

    pub fn merge_into(&self, other: &mut Self) {
        match self.current_thread {
            ThreadId::Id(id) => {
                let initial_thread_stack = mem::replace(
                    &mut other.initial_thread_stack,
                    self.initial_thread_stack.clone(),
                );
                match other.thread_stacks.entry(id) {
                    Entry::Occupied(mut entry) => {
                        initial_thread_stack.merge_into(entry.get_mut());
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(initial_thread_stack);
                    }
                }
            }
            ThreadId::Initial => {
                self.initial_thread_stack
                    .merge_into(&mut other.initial_thread_stack);
            }
            ThreadId::None => {}
        }
        if let ThreadId::Initial = other.current_thread {
            other.current_thread = self.current_thread;
        }
        self.thread_stacks.iter().for_each(|(&thread_id, stack)| {
            match other.thread_stacks.entry(thread_id) {
                Entry::Occupied(mut entry) => {
                    stack.merge_into(entry.get_mut());
                }
                Entry::Vacant(entry) => {
                    entry.insert(stack.clone());
                }
            }
        });
        other.thread_stacks.retain(|_, stack| !stack.exited);
    }
}

#[derive(Debug, Clone)]
struct CallStackEntry {
    vertex_index: usize,
    method_id: u64,
}

#[derive(Debug, Clone)]
struct ThreadTraceState {
    thread_id: u32,
    stack: Vec<CallStackEntry>,
    call_boxes: Vec<CallBox>,
    thread_line: ThreadLine,
}

impl ThreadTraceState {
    fn from_stack(thread_id: u32, stack: &StackState, start_time: u64, end_time: u64) -> Self {
        Self {
            thread_id,
            stack: stack
                .stack
                .iter()
                .map(|&method_id| CallStackEntry {
                    method_id,
                    vertex_index: usize::MAX,
                })
                .collect(),
            call_boxes: Vec::new(),
            thread_line: ThreadLine {
                start_time: encode_time(start_time),
                end_time: encode_time(end_time),
            },
        }
    }

    fn new(thread_id: u32, start_time: u64, end_time: u64) -> Self {
        Self {
            thread_id,
            stack: Vec::new(),
            call_boxes: Vec::new(),
            thread_line: ThreadLine {
                start_time: encode_time(start_time),
                end_time: encode_time(end_time),
            },
        }
    }

    fn into_thread_data(self) -> ThreadData {
        ThreadData {
            thread_id: self.thread_id,
            call_boxes: self.call_boxes,
            thread_line: self.thread_line,
        }
    }
}

pub struct SlowTrace {
    data: Vec<ThreadData>,
    max_depth: u32,
    end_time: u64,
    gc_events: Vec<u64>,
}

impl SlowTrace {
    pub fn trace(start_time: u64, fast_trace: &FastTrace, events: &[RRTraceEvent]) -> SlowTrace {
        let end_time = events.last().unwrap().timestamp();
        let mut max_depth = 0;
        let &FastTrace {
            ref thread_stacks,
            initial_thread_stack: _,
            current_thread,
            in_gc,
        } = fast_trace;
        let current_thread = match current_thread {
            ThreadId::None => u32::MAX,
            ThreadId::Initial => unreachable!(),
            ThreadId::Id(id) => id,
        };
        let mut gc_events = Vec::new();
        let mut call_stack = thread_stacks
            .iter()
            .map(|(&thread_id, stack)| {
                ThreadTraceState::from_stack(thread_id, stack, start_time, end_time)
            })
            .collect::<Vec<_>>();
        if !in_gc
            && let Some(ThreadTraceState {
                stack, call_boxes, ..
            }) = call_stack.get_mut(current_thread as usize)
        {
            call_boxes.reserve(stack.len());
            for (depth, entry) in stack.iter_mut().enumerate() {
                entry.vertex_index = call_boxes.len();
                let depth = depth as u32;
                max_depth = max_depth.max(depth);
                call_boxes.push(CallBox {
                    start_time: encode_time(start_time),
                    end_time: encode_time(end_time),
                    method_id: entry.method_id as u32,
                    depth,
                });
            }
        }
        let mut null_vec1 = Vec::new();
        let mut null_vec2 = Vec::new();
        let (mut current_stack, mut current_vertices) =
            call_stack.get_mut(current_thread as usize).map_or(
                (&mut null_vec1, &mut null_vec2),
                |ThreadTraceState {
                     stack, call_boxes, ..
                 }| { (stack, call_boxes) },
            );
        for event in events {
            match event.event_type() {
                RRTraceEventType::Call => {
                    let vertex_index = current_vertices.len();
                    let depth = current_stack.len() as u32;
                    current_stack.push(CallStackEntry {
                        vertex_index,
                        method_id: event.data(),
                    });
                    current_vertices.push(CallBox {
                        start_time: encode_time(event.timestamp()),
                        end_time: encode_time(end_time),
                        method_id: event.data() as u32,
                        depth,
                    });
                    max_depth = max_depth.max(depth);
                }
                RRTraceEventType::Return => {
                    while let Some(CallStackEntry {
                        vertex_index,
                        method_id,
                    }) = current_stack.pop()
                    {
                        current_vertices[vertex_index].end_time = encode_time(event.timestamp());
                        if method_id == event.data() {
                            break;
                        }
                    }
                }
                RRTraceEventType::GCStart => {
                    gc_events.push(event.timestamp());
                    for CallStackEntry { vertex_index, .. } in current_stack.iter_mut() {
                        current_vertices[*vertex_index].end_time = encode_time(event.timestamp());
                        *vertex_index = usize::MAX;
                    }
                }
                RRTraceEventType::GCEnd => {
                    gc_events.push(event.timestamp());
                    for (
                        depth,
                        &mut CallStackEntry {
                            ref mut vertex_index,
                            method_id,
                        },
                    ) in current_stack.iter_mut().enumerate()
                    {
                        let new_index = current_vertices.len();
                        let depth = depth as u32;
                        current_vertices.push(CallBox {
                            start_time: encode_time(event.timestamp()),
                            end_time: encode_time(end_time),
                            method_id: method_id as u32,
                            depth,
                        });
                        max_depth = max_depth.max(depth);
                        *vertex_index = new_index;
                    }
                }
                RRTraceEventType::ThreadSuspended => {
                    for CallStackEntry { vertex_index, .. } in current_stack.iter_mut() {
                        current_vertices[*vertex_index].end_time = encode_time(event.timestamp());
                        *vertex_index = usize::MAX;
                    }
                }
                RRTraceEventType::ThreadResume => {
                    let thread_id = event.data() as u32;
                    let index = call_stack.binary_search_by_key(
                        &thread_id,
                        |&ThreadTraceState { thread_id, .. }| thread_id,
                    );
                    if let Err(i) = index {
                        call_stack
                            .insert(i, ThreadTraceState::new(thread_id, start_time, end_time));
                    }
                    let ThreadTraceState {
                        stack: new_stack,
                        call_boxes: new_vertices,
                        ..
                    } = &mut call_stack[index.unwrap_or_else(convert::identity)];

                    for (
                        depth,
                        &mut CallStackEntry {
                            ref mut vertex_index,
                            method_id,
                        },
                    ) in new_stack.iter_mut().enumerate()
                    {
                        let new_index = new_vertices.len();
                        let depth = depth as u32;
                        new_vertices.push(CallBox {
                            start_time: encode_time(event.timestamp()),
                            end_time: encode_time(end_time),
                            method_id: method_id as u32,
                            depth,
                        });
                        max_depth = max_depth.max(depth);
                        *vertex_index = new_index;
                    }

                    (current_stack, current_vertices) = (new_stack, new_vertices);
                }
                RRTraceEventType::ThreadStart => {
                    let thread_id = event.data() as u32;
                    let index = call_stack.binary_search_by_key(
                        &thread_id,
                        |&ThreadTraceState { thread_id, .. }| thread_id,
                    );
                    let i = match index {
                        Ok(i) => {
                            call_stack[i].thread_line.start_time = encode_time(event.timestamp());
                            i
                        }
                        Err(i) => {
                            call_stack.insert(
                                i,
                                ThreadTraceState::new(thread_id, event.timestamp(), end_time),
                            );
                            i
                        }
                    };
                    let ThreadTraceState {
                        stack, call_boxes, ..
                    } = &mut call_stack[i];
                    current_stack = stack;
                    current_vertices = call_boxes;
                }
                RRTraceEventType::ThreadExit => {
                    let thread_id = event.data() as u32;
                    let index = call_stack.binary_search_by_key(
                        &thread_id,
                        |&ThreadTraceState { thread_id, .. }| thread_id,
                    );
                    let i = match index {
                        Ok(i) => {
                            call_stack[i].thread_line.end_time = encode_time(event.timestamp());
                            i
                        }
                        Err(i) => {
                            call_stack.insert(
                                i,
                                ThreadTraceState::new(thread_id, start_time, event.timestamp()),
                            );
                            i
                        }
                    };
                    let ThreadTraceState {
                        stack, call_boxes, ..
                    } = &mut call_stack[i];
                    current_stack = stack;
                    current_vertices = call_boxes;
                }
                RRTraceEventType::ThreadReady => {}
            }
        }
        SlowTrace {
            data: call_stack
                .into_iter()
                .map(ThreadTraceState::into_thread_data)
                .collect(),
            max_depth,
            end_time,
            gc_events,
        }
    }

    pub fn data(&self) -> &[ThreadData] {
        &self.data
    }

    pub fn gc_events(&self) -> &[u64] {
        &self.gc_events
    }

    pub fn end_time(&self) -> u64 {
        self.end_time
    }

    pub fn max_depth(&self) -> u32 {
        self.max_depth
    }
}
