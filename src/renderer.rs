use crate::trace_state::{CallBox, SlowTrace, TraceState};
use glam::{Mat4, Vec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
}

impl Vertex {
    const fn new(x: f32, y: f32, z: f32) -> Self {
        Self {
            position: [x, y, z],
        }
    }

    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            }],
        }
    }
}

impl CallBox {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CallBox>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Uint32x2,
                },
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Uint32x2,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Uint32,
                },
                wgpu::VertexAttribute {
                    offset: 20,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Uint32,
                },
            ],
        }
    }
}

const VERTICES: &[Vertex] = &[
    Vertex::new(0.0, 0.0, 0.2),
    Vertex::new(1.0, 0.0, 0.2),
    Vertex::new(0.0, 1.0, 0.2),
    Vertex::new(1.0, 1.0, 0.2),
    Vertex::new(0.0, 0.0, 0.8),
    Vertex::new(1.0, 0.0, 0.8),
    Vertex::new(0.0, 1.0, 0.8),
    Vertex::new(1.0, 1.0, 0.8),
];

const INDICES: &[u16] = &[
    0, 2, 3, 0, 3, 1, 1, 3, 7, 1, 7, 5, 5, 7, 6, 5, 6, 4, 4, 6, 2, 4, 2, 0, 2, 6, 7, 2, 7, 3,
];

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    base_time: [u32; 2],
    max_depth: u32,
    num_threads: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ThreadInfo {
    lane_id: u32,
}

struct SurfaceState {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    depth_texture: wgpu::TextureView,
}

pub struct Renderer {
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: wgpu::Adapter,
    surface_state: Option<SurfaceState>,
    shader: wgpu::ShaderModule,
    render_pipeline_layout: wgpu::PipelineLayout,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    num_indices: u32,
    camera_uniform: CameraUniform,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    lane_alignment: u32,
    trace_queue: Arc<crossbeam_queue::SegQueue<SlowTrace>>,
}

impl Renderer {
    pub fn new(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        trace_queue: Arc<crossbeam_queue::SegQueue<SlowTrace>>,
    ) -> Self {
        let limits = device.limits();
        let lane_alignment = limits.min_uniform_buffer_offset_alignment;
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));

        let camera_uniform = CameraUniform {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            base_time: [0, 0],
            max_depth: 0,
            num_threads: 0,
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        const MAX_LANES: u32 = 256;
        let thread_info_buffer_size = MAX_LANES * lane_alignment;
        let mut thread_info_data = vec![0u8; thread_info_buffer_size as usize];
        for i in 0..MAX_LANES {
            let offset = (i * lane_alignment) as usize;
            let info = ThreadInfo { lane_id: i };
            thread_info_data[offset..offset + 4].copy_from_slice(bytemuck::bytes_of(&info));
        }

        let thread_info_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Thread Info Buffer"),
            contents: &thread_info_data,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
                label: Some("camera_bind_group_layout"),
            });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &camera_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &thread_info_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<ThreadInfo>() as u64),
                    }),
                },
            ],
            label: Some("camera_bind_group"),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&camera_bind_group_layout],
                immediate_size: 0,
            });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });
        let num_indices = INDICES.len() as u32;

        Self {
            instance,
            device,
            queue,
            adapter,
            surface_state: None,
            shader,
            render_pipeline_layout,
            vertex_buffer,
            index_buffer,
            num_indices,
            camera_uniform,
            camera_buffer,
            camera_bind_group,
            lane_alignment,
            trace_queue,
        }
    }

    pub fn set_window(&mut self, window: std::sync::Arc<winit::window::Window>) {
        let size = window.inner_size();
        let surface = self.instance.create_surface(window).unwrap();

        let surface_caps = surface.get_capabilities(&self.adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&self.device, &config);

        let render_pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Render Pipeline"),
                layout: Some(&self.render_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Vertex::desc(), CallBox::desc()],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Front),
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                cache: None,
                multiview_mask: None,
            });

        let depth_texture = Self::create_depth_texture(&self.device, &config);

        self.surface_state = Some(SurfaceState {
            surface,
            config,
            render_pipeline,
            depth_texture,
        });
    }

    fn create_depth_texture(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
    ) -> wgpu::TextureView {
        let size = wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        };
        let desc = wgpu::TextureDescriptor {
            label: Some("Depth Texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let texture = device.create_texture(&desc);
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if let Some(state) = &mut self.surface_state {
            if new_size.width > 0 && new_size.height > 0 {
                state.config.width = new_size.width;
                state.config.height = new_size.height;
                state.surface.configure(&self.device, &state.config);
                state.depth_texture = Self::create_depth_texture(&self.device, &state.config);
            }
        }
    }

    pub(crate) fn sync(&self) -> bool {
        let mut updated = false;
        while let Some(trace) = self.trace_queue.pop() {
            updated = true;
            todo!();
        }
        updated
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let Some(state) = &self.surface_state else {
            return Ok(());
        };

        let aspect = if let Some(state) = &self.surface_state {
            state.config.width as f32 / state.config.height as f32
        } else {
            1.0
        };

        // カメラの更新
        let view = Mat4::look_at_rh(
            Vec3::new(-1.0, 1.0, 2.0), // eye
            Vec3::new(0.5, 0.5, 0.5),  // center
            Vec3::Y,                   // up
        );
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 10000.0);
        self.camera_uniform.view_proj = (proj * view).to_cols_array_2d();
        self.camera_uniform.base_time = trace_state.base_time();
        self.camera_uniform.max_depth = max_depth;
        self.camera_uniform.num_threads = trace_state.num_threads();

        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[self.camera_uniform]),
        );

        let output = state.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.01,
                            g: 0.02,
                            b: 0.05,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &state.depth_texture,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });

            render_pass.set_pipeline(&state.render_pipeline);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

            let num_indices = self.num_indices;
            let camera_bind_group = &self.camera_bind_group;
            let lane_alignment = self.lane_alignment;

            trace_state.read_vertices(|lane, buffer, len| {
                if len == 0 {
                    return;
                }
                let offset = lane as u32 * lane_alignment;
                render_pass.set_bind_group(0, camera_bind_group, &[offset]);
                render_pass.set_vertex_buffer(1, buffer.slice(..));
                render_pass.draw_indexed(0..num_indices, 0, 0..len as u32);
            });
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
