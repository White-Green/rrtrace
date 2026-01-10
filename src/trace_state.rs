use crate::ringbuffer::{RRProfTraceEvent, RRProfTraceEventType};
use smallvec::SmallVec;
use std::convert;
use std::fmt::Debug;

#[repr(C)]
#[derive(Copy, Default, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CallBox {
    start_time: [u32; 2],
    end_time: [u32; 2],
    method_id: u32,
    depth: u32,
}

pub const VISIBLE_DURATION: u64 = 1_000_000_000 * 30;

pub fn encode_time(time: u64) -> [u32; 2] {
    [
        (time & 0x7fffffff) as u32,
        ((time >> 31) & 0xffffffff) as u32,
    ]
}

#[derive(Debug, Clone)]
pub struct FastTrace {
    thread_stacks: SmallVec<[(u32, SmallVec<[u64; 4]>); 1]>,
    current_thread: u32,
    in_gc: bool,
}

impl FastTrace {
    pub fn new() -> FastTrace {
        FastTrace {
            thread_stacks: smallvec::smallvec![(0, SmallVec::new())],
            current_thread: 0,
            in_gc: false,
        }
    }
    pub fn process_events(&mut self, events: &[RRProfTraceEvent]) {
        self.in_gc = false;
        for &event in events {
            match event.event_type() {
                RRProfTraceEventType::Call => {
                    let method_id = event.data();
                    self.thread_stacks[self.current_thread as usize]
                        .1
                        .push(method_id);
                }
                RRProfTraceEventType::Return => {
                    let method_id = event.data();
                    let stack = &mut self.thread_stacks[self.current_thread as usize].1;
                    while stack.pop().is_some_and(|m| m != method_id) {}
                }
                RRProfTraceEventType::ThreadSuspended => {
                    self.current_thread = u32::MAX;
                }
                RRProfTraceEventType::ThreadResume => {
                    let thread_id = event.data() as u32;
                    let index = match self
                        .thread_stacks
                        .binary_search_by_key(&thread_id, |&(thread_id, _)| thread_id)
                    {
                        Ok(index) => index,
                        Err(index) => {
                            self.thread_stacks
                                .insert(index, (thread_id, SmallVec::new()));
                            index
                        }
                    };
                    self.current_thread = index as u32;
                }
                RRProfTraceEventType::ThreadExit => {
                    let thread_id = event.data() as u32;
                    if let Ok(index) = self
                        .thread_stacks
                        .binary_search_by_key(&thread_id, |&(thread_id, _)| thread_id)
                    {
                        self.thread_stacks.remove(index);
                    };
                }
                RRProfTraceEventType::GCStart
                | RRProfTraceEventType::GCEnd
                | RRProfTraceEventType::ThreadStart
                | RRProfTraceEventType::ThreadReady => {}
            }
        }
        self.in_gc = events.last().unwrap().event_type() == RRProfTraceEventType::GCStart;
    }
}

#[derive(Debug, Clone)]
struct CallStackEntry {
    vertex_index: usize,
    method_id: u64,
}

pub struct SlowTrace {
    data: Vec<(u32, Vec<CallStackEntry>, Vec<CallBox>)>,
    max_depth: u32,
    end_time: u64,
}

impl SlowTrace {
    pub fn trace(start_time: u64, fast_trace: FastTrace, events: &[RRProfTraceEvent]) -> SlowTrace {
        let end_time = events.last().unwrap().timestamp();
        let mut max_depth = 0;
        let FastTrace {
            thread_stacks,
            current_thread,
            in_gc,
        } = fast_trace;
        let mut call_stack = thread_stacks
            .into_iter()
            .map(|(thread_id, stack)| {
                (
                    thread_id,
                    stack
                        .into_iter()
                        .map(|method_id| CallStackEntry {
                            method_id,
                            vertex_index: usize::MAX,
                        })
                        .collect::<Vec<_>>(),
                    Vec::new(),
                )
            })
            .collect::<Vec<_>>();
        if !in_gc && let Some((_, stack, vertices)) = call_stack.get_mut(current_thread as usize) {
            vertices.reserve(stack.len());
            for (depth, entry) in stack.iter_mut().enumerate() {
                entry.vertex_index = vertices.len();
                let depth = depth as u32;
                max_depth = max_depth.max(depth);
                vertices.push(CallBox {
                    start_time: encode_time(start_time),
                    end_time: encode_time(end_time),
                    method_id: entry.method_id as u32,
                    depth,
                });
            }
        }
        let mut null_vec1 = Vec::new();
        let mut null_vec2 = Vec::new();
        let (mut current_stack, mut current_vertices) = call_stack
            .get_mut(current_thread as usize)
            .map_or((&mut null_vec1, &mut null_vec2), |(_, stack, vertices)| {
                (stack, vertices)
            });
        for event in events {
            match event.event_type() {
                RRProfTraceEventType::Call => {
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
                RRProfTraceEventType::Return => {
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
                RRProfTraceEventType::GCStart => {
                    for CallStackEntry { vertex_index, .. } in current_stack.iter_mut() {
                        current_vertices[*vertex_index].end_time = encode_time(event.timestamp());
                        *vertex_index = usize::MAX;
                    }
                }
                RRProfTraceEventType::GCEnd => {
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
                RRProfTraceEventType::ThreadSuspended => {
                    for CallStackEntry { vertex_index, .. } in current_stack.iter_mut() {
                        current_vertices[*vertex_index].end_time = encode_time(event.timestamp());
                        *vertex_index = usize::MAX;
                    }
                }
                RRProfTraceEventType::ThreadResume => {
                    let thread_id = event.data() as u32;
                    let index = call_stack.binary_search_by_key(&thread_id, |&(tid, _, _)| tid);
                    if let Err(i) = index {
                        call_stack.insert(i, (thread_id, Vec::new(), Vec::new()));
                    }
                    let (_, new_stack, new_vertices) =
                        &mut call_stack[index.unwrap_or_else(convert::identity)];

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
                RRProfTraceEventType::ThreadExit
                | RRProfTraceEventType::ThreadStart
                | RRProfTraceEventType::ThreadReady => {}
            }
        }
        SlowTrace {
            data: call_stack,
            max_depth,
            end_time,
        }
    }

    pub fn data(&self) -> impl Iterator<Item = (u32, &[CallBox])> {
        self.data
            .iter()
            .map(|&(thread_id, _, ref call_box)| (thread_id, call_box.as_slice()))
    }

    pub fn end_time(&self) -> u64 {
        self.end_time
    }

    pub fn max_depth(&self) -> u32 {
        self.max_depth
    }
}
