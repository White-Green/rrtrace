use crate::ringbuffer::{EventRingBuffer, RRProfTraceEvent};
use crate::trace_state::TraceState;
use std::ffi::CString;
use std::sync::{Arc, Mutex};
use std::{env, mem, thread};
use winit::application::ApplicationHandler;
use winit::event::*;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::Window;
use crate::renderer::Renderer;

mod renderer;
mod ringbuffer;
#[cfg_attr(unix, path = "shm_unix.rs")]
#[cfg_attr(windows, path = "shm_windows.rs")]
mod shm;
mod trace_state;

struct App {
    window: Option<Arc<Window>>,
    renderer: Renderer,
    trace_state: Arc<Mutex<TraceState>>,
}

impl App {
    fn new(renderer: Renderer, trace_state: Arc<Mutex<TraceState>>) -> Self {
        Self {
            window: None,
            renderer,
            trace_state,
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
            WindowEvent::RedrawRequested => {
                let mut trace_state = self.trace_state.lock().unwrap();
                match self.renderer.render(&mut trace_state) {
                    Ok(_) => {}
                    Err(wgpu::SurfaceError::Lost) => self.renderer.resize(window.inner_size()),
                    Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                    Err(e) => eprintln!("{:?}", e),
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {}
}

fn main() {
    assert_eq!(env::args().len(), 2, "Usage: rrprof <shm_name>");
    let shm_name = env::args().nth(1).unwrap();

    let (instance, adapter, device, queue) = pollster::block_on(init_gpu());
    let trace_state = Arc::new(Mutex::new(TraceState::new(device.clone(), queue.clone())));
    let event_queue = Arc::new(crossbeam_queue::SegQueue::new());
    thread::Builder::new()
        .name("queue pipe".to_owned())
        .spawn(queue_pipe_thread(shm_name, Arc::clone(&event_queue)))
        .unwrap();
    thread::Builder::new()
        .name("trace".to_owned())
        .spawn(trace_thread(Arc::clone(&trace_state), event_queue))
        .unwrap();

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(Renderer::new(instance, adapter, device, queue), trace_state);
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
    state: Arc<Mutex<TraceState>>,
    event_queue: Arc<crossbeam_queue::SegQueue<Vec<RRProfTraceEvent>>>,
) -> impl Fn() + Send + 'static {
    move || loop {
        let Some(events) = event_queue.pop() else {
            continue;
        };
        let mut state = state.lock().unwrap();
        state.process_events(&events);
    }
}
