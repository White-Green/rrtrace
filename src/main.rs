use crate::renderer::Renderer;
use crate::ringbuffer::{EventRingBuffer, RRProfTraceEvent};
use crate::trace_state::{FastTrace, SlowTrace, TraceState, VISIBLE_DURATION};
use std::ffi::CString;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, atomic};
use std::{env, mem, thread};
use winit::application::ApplicationHandler;
use winit::event::*;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::Window;

mod renderer;
mod ringbuffer;
#[cfg_attr(unix, path = "shm_unix.rs")]
#[cfg_attr(windows, path = "shm_windows.rs")]
mod shm;
mod trace_state;

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
                .create_window(Window::default_attributes().with_title("rrprof visualizer"))
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
        let updated = self.renderer.sync();
        if updated && let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn main() {
    assert_eq!(env::args().len(), 2, "Usage: rrprof <shm_name>");
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
    event_queue: Arc<crossbeam_queue::SegQueue<Vec<RRProfTraceEvent>>>,
) -> impl FnOnce() + Send + 'static {
    move || {
        let shm = unsafe {
            shm::SharedMemory::open(
                CString::new(shm_name).unwrap(),
                mem::size_of::<ringbuffer::RRProfEventRingBuffer>(),
            )
        };
        let mut ringbuffer = unsafe { EventRingBuffer::new(shm.as_ptr(), move || drop(shm)) };
        let mut buffer = vec![Default::default(); 65536];
        loop {
            let count = ringbuffer.read(&mut buffer);
            if count > 0 {
                buffer.truncate(count);
                event_queue.push(buffer.clone());
                buffer.resize_with(65536, Default::default);
            }
        }
    }
}
fn trace_thread(
    event_queue: Arc<crossbeam_queue::SegQueue<Vec<RRProfTraceEvent>>>,
    result_queue: Arc<crossbeam_queue::SegQueue<SlowTrace>>,
) -> impl FnOnce() + Send + 'static {
    move || {
        static LATEST_END_TIME: AtomicU64 = AtomicU64::new(0);
        let mut start_time = 0u64;
        let mut fast_trace = FastTrace::new();
        loop {
            let Some(events) = event_queue.pop() else {
                continue;
            };
            rayon_core::spawn({
                let fast_trace = fast_trace.clone();
                let events = events.clone();
                let result_queue = result_queue.clone();
                move || {
                    if start_time + VISIBLE_DURATION
                        < LATEST_END_TIME.load(atomic::Ordering::Relaxed)
                    {
                        return;
                    }
                    let slow_trace = SlowTrace::trace(start_time, fast_trace, &events);
                    result_queue.push(slow_trace);
                }
            });
            fast_trace.process_events(&events);
            let end_time = events.last().unwrap().timestamp();
            LATEST_END_TIME.store(end_time, atomic::Ordering::Relaxed);
            start_time = end_time;
        }
    }
}
