use crate::object_scatter::ObjectScatter;
use crate::renderer::Renderer;
use crate::ringbuffer::{EventRingBuffer, RRTraceEvent};
use crate::trace_state::{FastTrace, SlowTrace};
use crate::universal_notifier::UniversalNotifier;
use std::ffi::CString;
use std::num::NonZeroUsize;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Instant;
use std::{env, mem, thread};
use winit::application::ApplicationHandler;
use winit::event::*;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::Window;

mod object_scatter;
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
        loop {
            let count = ringbuffer.read(&mut buffer);
            if count > 0 {
                event_queue.push(Arc::from(&buffer[..count]));
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
        let (second_stage_sender, second_stage_receivers) =
            ObjectScatter::<(u64, Arc<FastTrace>, Arc<[RRTraceEvent]>)>::new(
                parallel_trace_threads,
            );
        struct Persistent {
            second_trace_sender: ObjectScatter<(u64, Arc<FastTrace>, Arc<[RRTraceEvent]>)>,
            trace: Option<Arc<FastTrace>>,
        }
        let first_stage_event_queue = Arc::new(crossbeam_queue::SegQueue::<(
            Receiver<Persistent>,
            SyncSender<Persistent>,
            u64,
            Arc<[RRTraceEvent]>,
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
                            if let Some((receiver, sender, start_time, events)) =
                                first_stage_event_queue.pop()
                            {
                                let mut trace = FastTrace::from_events(&events);
                                let mut persistent = receiver.recv().unwrap();
                                let t = persistent
                                    .trace
                                    .as_ref()
                                    .map_or_else(|| Arc::new(FastTrace::default()), Arc::clone);
                                persistent.second_trace_sender.send((start_time, t, events));
                                match &persistent.trace {
                                    None => trace.mark_as_first(),
                                    Some(t) => t.merge_into(&mut trace),
                                }
                                let trace = Arc::new(trace);
                                sender
                                    .send(Persistent {
                                        trace: Some(trace),
                                        ..persistent
                                    })
                                    .unwrap();
                                universal_notifier.notify();
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
        let mut start_time = 0u64;
        let (s, mut receiver) = mpsc::sync_channel(1);
        s.send(Persistent {
            second_trace_sender: second_stage_sender,
            trace: None,
        })
        .unwrap();
        loop {
            let Some(events) = event_queue.pop() else {
                continue;
            };
            let end_time = events.last().unwrap().timestamp();
            let (sender, r) = mpsc::sync_channel(1);
            first_stage_event_queue.push((
                mem::replace(&mut receiver, r),
                sender,
                start_time,
                events,
            ));
            universal_notifier.notify();
            start_time = end_time;
        }
    }
}
