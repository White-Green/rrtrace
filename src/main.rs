use crate::ringbuffer::{EventRingBuffer, RRProfTraceEvent};
use std::env;
use std::ffi::CString;
use std::mem;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ControlFlow, EventLoop},
    window::Window,
};

mod ringbuffer;
mod shm;
mod visualizer;
mod renderer;

struct App {
    shm_name: String,
    window: Option<Arc<Window>>,
    renderer: Option<renderer::Renderer>,
    trace_state: visualizer::TraceState,
    ringbuffer: Option<EventRingBuffer>,
    event_buf: [RRProfTraceEvent; 1024],
}

impl App {
    fn new(shm_name: String) -> Self {
        Self {
            shm_name,
            window: None,
            renderer: None,
            trace_state: visualizer::TraceState::new(),
            ringbuffer: None,
            event_buf: [RRProfTraceEvent::default(); 1024],
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let window = Arc::new(event_loop.create_window(Window::default_attributes().with_title("rrprof visualizer")).unwrap());
        self.window = Some(window.clone());

        let shm = unsafe {
            shm::SharedMemory::open(
                CString::new(self.shm_name.clone()).unwrap(),
                mem::size_of::<ringbuffer::RRProfEventRingBuffer>(),
            )
        };
        self.ringbuffer = Some(unsafe { EventRingBuffer::new(shm.as_ptr(), move || drop(shm)) });

        let renderer = pollster::block_on(renderer::Renderer::new(window));
        self.renderer = Some(renderer);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let window = self.window.as_ref().unwrap();
        let renderer = self.renderer.as_mut().unwrap();

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
                renderer.resize(physical_size);
            }
            WindowEvent::RedrawRequested => {
                renderer.update(&self.trace_state);
                match renderer.render() {
                    Ok(_) => {}
                    Err(wgpu::SurfaceError::Lost) => renderer.resize(window.inner_size()),
                    Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                    Err(e) => eprintln!("{:?}", e),
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(ringbuffer) = &mut self.ringbuffer {
            let count = ringbuffer.read(&mut self.event_buf);
            if count > 0 {
                for i in 0..count {
                    self.trace_state.process_event(&self.event_buf[i]);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }
}

fn main() {
    env_logger::init();
    assert_eq!(env::args().len(), 2, "Usage: rrprof <shm_name>");
    let shm_name = env::args().nth(1).unwrap();

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(shm_name);
    event_loop.run_app(&mut app).unwrap();
}
