use glam::{Mat4, Vec3};
use std::collections::HashMap;
use crate::ringbuffer::RRProfTraceEvent;

const EVENT_TYPE_MASK: u64 = 0xF000000000000000;
const EVENT_TYPE_CALL: u64 = 0x0000000000000000;
const EVENT_TYPE_RETURN: u64 = 0x1000000000000000;
const EVENT_TYPE_GC_START: u64 = 0x2000000000000000;
const EVENT_TYPE_GC_END: u64 = 0x3000000000000000;
const EVENT_TYPE_THREAD_START: u64 = 0x4000000000000000;
const EVENT_TYPE_THREAD_READY: u64 = 0x5000000000000000;
const EVENT_TYPE_THREAD_SUSPENDED: u64 = 0x6000000000000000;
const EVENT_TYPE_THREAD_RESUME: u64 = 0x7000000000000000;
const EVENT_TYPE_THREAD_EXIT: u64 = 0x8000000000000000;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InstanceRaw {
    model: [[f32; 4]; 4],
    color: [f32; 4],
}

impl InstanceRaw {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<InstanceRaw>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 12]>() as wgpu::BufferAddress,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 16]>() as wgpu::BufferAddress,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

struct CallInfo {
    start_time: u64,
    method_id: u64,
    stack_depth: u32,
}

pub struct TraceState {
    thread_stacks: HashMap<u32, Vec<CallInfo>>,
    instances: Vec<InstanceRaw>,
    current_thread_id: u32,
    thread_to_lane: HashMap<u32, u32>,
    next_lane: u32,
    base_time: Option<u64>,
}

impl TraceState {
    pub fn new() -> Self {
        Self {
            thread_stacks: HashMap::new(),
            instances: Vec::new(),
            current_thread_id: 0,
            thread_to_lane: HashMap::new(),
            next_lane: 0,
            base_time: None,
        }
    }

    pub fn process_event(&mut self, event: &RRProfTraceEvent) {
        let timestamp = event.timestamp_and_event_type & !EVENT_TYPE_MASK;
        let event_type = event.timestamp_and_event_type & EVENT_TYPE_MASK;

        if self.base_time.is_none() {
            self.base_time = Some(timestamp);
        }
        let rel_time = timestamp - self.base_time.unwrap();

        match event_type {
            EVENT_TYPE_CALL => {
                let stack = self.thread_stacks.entry(self.current_thread_id).or_default();
                stack.push(CallInfo {
                    start_time: rel_time,
                    method_id: event.data,
                    stack_depth: stack.len() as u32,
                });
            }
            EVENT_TYPE_RETURN => {
                if let Some(stack) = self.thread_stacks.get_mut(&self.current_thread_id) {
                    if let Some(call) = stack.pop() {
                        self.add_instance(call, rel_time, self.current_thread_id);
                    }
                }
            }
            EVENT_TYPE_THREAD_START | EVENT_TYPE_THREAD_READY | EVENT_TYPE_THREAD_SUSPENDED | EVENT_TYPE_THREAD_RESUME | EVENT_TYPE_THREAD_EXIT => {
                self.current_thread_id = event.data as u32;
                if !self.thread_to_lane.contains_key(&self.current_thread_id) {
                    self.thread_to_lane.insert(self.current_thread_id, self.next_lane);
                    self.next_lane += 1;
                }
            }
            _ => {}
        }
    }

    fn add_instance(&mut self, call: CallInfo, end_time: u64, thread_id: u32) {
        let lane = *self.thread_to_lane.get(&thread_id).unwrap_or(&0) as f32;
        let start_x = call.start_time as f32 / 1_000_000.0; // ms
        let end_x = end_time as f32 / 1_000_000.0;
        let duration = end_x - start_x;

        if duration < 0.01 { return; } // 小さすぎるものはスキップ

        let color = self.method_id_to_color(call.method_id);

        // X: Time, Y: Thread Lane, Z: Stack Depth
        let transform = Mat4::from_translation(Vec3::new(start_x, lane, call.stack_depth as f32))
            * Mat4::from_scale(Vec3::new(duration, 0.8, 0.8));

        self.instances.push(InstanceRaw {
            model: transform.to_cols_array_2d(),
            color,
        });
    }

    fn method_id_to_color(&self, method_id: u64) -> [f32; 4] {
        let r = ((method_id * 123) % 255) as f32 / 255.0;
        let g = ((method_id * 456) % 255) as f32 / 255.0;
        let b = ((method_id * 789) % 255) as f32 / 255.0;
        [r, g, b, 1.0]
    }

    pub fn instances(&self) -> &[InstanceRaw] {
        &self.instances
    }
}
