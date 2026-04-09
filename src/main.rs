use crate::object_scatter::ObjectScatter;
use crate::oneshot_channel::{OneshotReceiver, OneshotSender};
use crate::renderer::Renderer;
use crate::ringbuffer::{EventRingBuffer, RRTraceEvent};
use crate::trace_state::{FastTrace, SlowTrace};
use crate::universal_notifier::UniversalNotifier;
use std::collections::VecDeque;
use std::ffi::CString;
use std::num::NonZeroUsize;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use std::{env, mem, thread};
use winit::application::ApplicationHandler;
use winit::event::*;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::Window;

mod object_scatter;
mod oneshot_channel;
mod renderer;
mod ringbuffer;
#[cfg_attr(unix, path = "shm_unix.rs")]
#[cfg_attr(windows, path = "shm_windows.rs")]
mod shm;
mod trace_state;
mod universal_notifier;

struct App {
    window: Option<Arc<Window>>,
    renderer: Renderer,
}

impl App {
    fn new(renderer: Renderer) -> Self {
        Self {
            window: None,
            renderer,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("rrtrace visualizer"))
                .unwrap(),
        );
        self.renderer.set_window(window.clone());
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state: ElementState::Pressed,
                        logical_key: winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape),
                        ..
                    },
                ..
            } => event_loop.exit(),
            WindowEvent::Resized(physical_size) => {
                self.renderer.resize(physical_size);
            }
            WindowEvent::RedrawRequested => match self.renderer.render() {
                Ok(_) => {}
                Err(wgpu::SurfaceError::Lost) => self.renderer.resize(window.inner_size()),
                Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                Err(e) => eprintln!("{:?}", e),
            },
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        self.renderer.sync();
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

static BASE_TIME: OnceLock<Instant> = OnceLock::new();

fn main() {
    BASE_TIME.set(Instant::now()).unwrap();
    assert_eq!(env::args().len(), 2, "Usage: rrtrace <shm_name>");
    let shm_name = env::args().nth(1).unwrap();

    let (instance, adapter, device, queue) = pollster::block_on(init_gpu());
    let event_queue = Arc::new(crossbeam_queue::SegQueue::new());
    let result_queue = Arc::new(crossbeam_queue::SegQueue::new());
    thread::Builder::new()
        .name("queue pipe".to_owned())
        .spawn(queue_pipe_thread(shm_name, Arc::clone(&event_queue)))
        .unwrap();
    thread::Builder::new()
        .name("trace".to_owned())
        .spawn(trace_thread(
            Arc::clone(&event_queue),
            Arc::clone(&result_queue),
        ))
        .unwrap();

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(Renderer::new(
        instance,
        adapter,
        device,
        queue,
        result_queue,
    ));
    event_loop.run_app(&mut app).unwrap();
}

async fn init_gpu() -> (wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .unwrap();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: Default::default(),
            memory_hints: Default::default(),
            trace: Default::default(),
        })
        .await
        .unwrap();
    (instance, adapter, device, queue)
}

fn queue_pipe_thread(
    shm_name: String,
    event_queue: Arc<crossbeam_queue::SegQueue<Arc<[RRTraceEvent]>>>,
) -> impl FnOnce() + Send + 'static {
    move || {
        let shm = unsafe {
            shm::SharedMemory::open(
                CString::new(shm_name).unwrap(),
                mem::size_of::<ringbuffer::RRTraceEventRingBuffer>(),
            )
        };
        let mut ringbuffer = unsafe { EventRingBuffer::new(shm.as_ptr(), move || drop(shm)) };
        let mut buffer = vec![Default::default(); 65536];
        let mut offset = 0;
        let mut before_send_time = 0;
        loop {
            let count = ringbuffer.read(&mut buffer[offset..]);
            if count > 0 {
                offset += count;
                let last_time = buffer[offset - 1].timestamp();
                if offset >= 1024 || last_time.saturating_sub(before_send_time) > 1_000_000 {
                    event_queue.push(Arc::from(&buffer[..offset]));
                    offset = 0;
                    before_send_time = last_time;
                }
            }
        }
    }
}
fn trace_thread(
    event_queue: Arc<crossbeam_queue::SegQueue<Arc<[RRTraceEvent]>>>,
    result_queue: Arc<crossbeam_queue::SegQueue<SlowTrace>>,
) -> impl FnOnce() + Send + 'static {
    move || {
        let parallel_trace_threads = thread::available_parallelism()
            .map_or(0, NonZeroUsize::get)
            .saturating_sub(2)
            .max(1);

        let universal_notifier = UniversalNotifier::new();
        let (mut second_stage_sender, second_stage_receivers) =
            ObjectScatter::<(u64, Arc<FastTrace>, Arc<[RRTraceEvent]>)>::new(
                parallel_trace_threads,
            );
        let first_stage_event_queue = Arc::new(crossbeam_queue::SegQueue::<(
            Arc<[RRTraceEvent]>,
            OneshotSender<FastTrace>,
        )>::new());
        second_stage_receivers
            .enumerate()
            .for_each(|(i, mut second_stage_receiver)| {
                let universal_notifier = universal_notifier.clone();
                let first_stage_event_queue = Arc::clone(&first_stage_event_queue);
                let result_queue = Arc::clone(&result_queue);
                thread::Builder::new()
                    .name(format!("slow_trace_thread_{}", i))
                    .spawn(move || {
                        loop {
                            let v = universal_notifier.value();
                            if let Some((events, result_slot)) = first_stage_event_queue.pop() {
                                let trace = FastTrace::from_events(&events);
                                result_slot.send(trace);
                                continue;
                            }
                            if let Some(data) = second_stage_receiver.try_receive() {
                                let (start_time, fast_trace, events) = *data;
                                let trace = SlowTrace::trace(start_time, &fast_trace, &events);
                                result_queue.push(trace);
                                continue;
                            }
                            universal_notifier.wait(v);
                        }
                    })
                    .unwrap();
            });
        let mut first = true;
        let mut start_time = 0u64;
        let mut local_event_queue = VecDeque::new();
        let mut first_stage_result_queue = VecDeque::new();
        let mut first_stage_results = VecDeque::new();
        let mut first_stage_result = None::<OneshotReceiver<FastTrace>>;
        let mut trace_accumulate = None::<Arc<FastTrace>>;
        loop {
            if first_stage_result.is_none()
                && let Some(receiver) = first_stage_result_queue.pop_front()
            {
                first_stage_result = Some(receiver);
            }
            if let Some(receiver) = first_stage_result.take() {
                match receiver.try_receive() {
                    Ok(mut trace) => {
                        match trace_accumulate.take() {
                            None => trace.mark_as_first(),
                            Some(acc) => acc.merge_into(&mut trace),
                        }
                        let trace = Arc::new(trace);
                        trace_accumulate = Some(Arc::clone(&trace));
                        first_stage_results.push_back(trace);
                        first_stage_result = first_stage_result_queue.pop_front();
                    }
                    Err(receiver) => first_stage_result = Some(receiver),
                };
            }
            if !first_stage_results.is_empty() && !local_event_queue.is_empty() {
                let trace = first_stage_results.pop_front().unwrap();
                let (start_time, events) = local_event_queue.pop_front().unwrap();
                second_stage_sender.send((start_time, trace, events));
                universal_notifier.notify();
            }
            if let Some(events) = event_queue.pop() {
                let end_time = events.last().unwrap().timestamp();
                let (sender, receiver) = oneshot_channel::channel();
                first_stage_event_queue.push((Arc::clone(&events), sender));
                universal_notifier.notify();
                first_stage_result_queue.push_back(receiver);
                if !mem::replace(&mut first, false) {
                    local_event_queue.push_back((start_time, events));
                }
                start_time = end_time;
            }
        }
    }
}
