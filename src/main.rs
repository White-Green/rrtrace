use crate::ringbuffer::{EventRingBuffer, RRProfTraceEvent};
use std::ffi::CString;
use std::sync::Arc;
use std::{env, mem};
use winit::application::ApplicationHandler;
use winit::event::*;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::Window;

mod renderer;
mod ringbuffer;
#[cfg_attr(unix, path = "shm_unix.rs")]
#[cfg_attr(windows, path = "shm_windows.rs")]
mod shm;

struct App {
    window: Option<Arc<Window>>,
    renderer: renderer::Renderer,
    ringbuffer: EventRingBuffer,
    event_buf: Vec<RRProfTraceEvent>,
}

impl App {
    fn new(ringbuffer: EventRingBuffer) -> Self {
        let renderer = pollster::block_on(renderer::Renderer::new());
        Self {
            window: None,
            renderer,
            ringbuffer,
            event_buf: vec![RRProfTraceEvent::default(); 65536],
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
                match self.renderer.render() {
                    Ok(_) => {}
                    Err(wgpu::SurfaceError::Lost) => self.renderer.resize(window.inner_size()),
                    Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                    Err(e) => eprintln!("{:?}", e),
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        let count = self.ringbuffer.read(&mut self.event_buf);
        if count > 0 {
            self.renderer.process_events(&self.event_buf[..count]);
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

fn main() {
    assert_eq!(env::args().len(), 2, "Usage: rrprof <shm_name>");
    let shm_name = env::args().nth(1).unwrap();

    let shm = unsafe {
        shm::SharedMemory::open(
            CString::new(shm_name).unwrap(),
            mem::size_of::<ringbuffer::RRProfEventRingBuffer>(),
        )
    };
    let ringbuffer = unsafe { EventRingBuffer::new(shm.as_ptr(), move || drop(shm)) };

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(ringbuffer);
    event_loop.run_app(&mut app).unwrap();
}
