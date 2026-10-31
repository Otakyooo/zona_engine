//! zn-render: рендерер на wgpu (Vulkan primary, DX12/DX11 fallback).

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;
use winit::window::Window;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    pub uv: [f32; 2],
}

pub struct Mesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
}

pub struct Texture {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
}

#[derive(Clone, Copy)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_y: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl Camera {
    pub fn view_proj(&self) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye, self.target, self.up);
        let proj = Mat4::perspective_rh(self.fov_y, self.aspect, self.near, self.far);
        proj * view
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 3],
    _pad0: f32,
    ambient: f32,
    _pad_before: [f32; 3],
    _pad1: [f32; 3],
    _pad2: f32,
    _pad4: [f32; 4],
    camera_pos: [f32; 3],
    _pad3: f32,
}

pub struct Renderer {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub size: (u32, u32),
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_bgl: wgpu::BindGroupLayout,
    default_texture: Texture,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

impl Renderer {
    pub async fn new(window: std::sync::Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();
        let backend_str = std::env::var("ZONA_BACKEND").unwrap_or_default();
        let backends = if backend_str == "dx12" {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::VULKAN | wgpu::Backends::DX12
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or("no adapter")?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    label: Some("device"),
                    memory_hints: Default::default(),
                },
                None,
            )
            .await
            .map_err(|e| e.to_string())?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // uniform buffer
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // bind group layouts
        let uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniform bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let texture_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniform bg"),
            layout: &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // default white texture
        let default_texture =
            create_texture_from_rgba(&device, &queue, &[255, 255, 255, 255], 1, 1, &texture_bgl);

        // shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline layout"),
            bind_group_layouts: &[&uniform_bgl, &texture_bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 12,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 24,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 36,
                            shader_location: 3,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
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
            multiview: None,
            cache: None,
        });

        let (depth_texture, depth_view) = create_depth(&device, &config);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size: (size.width, size.height),
            pipeline,
            uniform_buffer,
            uniform_bind_group,
            texture_bgl,
            default_texture,
            depth_texture,
            depth_view,
        })
    }

    pub fn create_mesh(&self, vertices: &[Vertex], indices: &[u32]) -> Mesh {
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vb"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ib"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        Mesh {
            vertex_buffer,
            index_buffer,
            num_indices: indices.len() as u32,
        }
    }

    pub fn create_texture(&self, rgba: &[u8], width: u32, height: u32) -> Texture {
        create_texture_from_rgba(
            &self.device,
            &self.queue,
            rgba,
            width,
            height,
            &self.texture_bgl,
        )
    }

    pub fn create_texture_mipped(
        &self,
        mip_levels: &[Vec<u8>],
        width: u32,
        height: u32,
    ) -> Texture {
        create_texture_mipped_from_levels(
            &self.device,
            &self.queue,
            mip_levels,
            width,
            height,
            &self.texture_bgl,
        )
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.size = (width, height);
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        let (dt, dv) = create_depth(&self.device, &self.config);
        self.depth_texture = dt;
        self.depth_view = dv;
    }

    pub fn render(
        &mut self,
        meshes: &[(&Mesh, Option<&Texture>)],
        camera: &Camera,
    ) -> Result<(), wgpu::SurfaceError> {
        let ld = Vec3::new(0.5, 1.0, 0.3).normalize();
        let uniforms = Uniforms {
            view_proj: camera.view_proj().to_cols_array_2d(),
            light_dir: [ld.x, ld.y, ld.z],
            _pad0: 0.0,
            ambient: 0.3,
            _pad_before: [0.0; 3],
            _pad1: [0.0; 3],
            _pad2: 0.0,
            _pad4: [0.0; 4],
            camera_pos: [camera.eye.x, camera.eye.y, camera.eye.z],
            _pad3: 0.0,
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.53,
                            g: 0.81,
                            b: 0.92,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            for (mesh, tex) in meshes {
                let t = tex.unwrap_or(&self.default_texture);
                pass.set_bind_group(1, &t.bind_group, &[]);
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.num_indices, 0, 0..1);
            }
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }
}

fn create_texture_from_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    rgba: &[u8],
    width: u32,
    height: u32,
    bgl: &wgpu::BindGroupLayout,
) -> Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tex"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tex bg"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    Texture {
        view,
        sampler,
        bind_group,
    }
}

fn create_texture_mipped_from_levels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mip_levels: &[Vec<u8>],
    width: u32,
    height: u32,
    bgl: &wgpu::BindGroupLayout,
) -> Texture {
    let mip_count = mip_levels.len().max(1) as u32;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tex_mipped"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut w = width;
    let mut h = height;
    for (i, level) in mip_levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: i as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            level,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
        );
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tex bg"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    Texture {
        view,
        sampler,
        bind_group,
    }
}
fn create_depth(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

const SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    light_dir: vec3<f32>,
    _pad0: f32,
    ambient: f32,
    _pad_before: vec3<f32>,
    _pad1: vec3<f32>,
    _pad2: f32,
    camera_pos: vec3<f32>,
    _pad3: f32,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(1) @binding(1) var s_diffuse: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
}
struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) world_pos: vec3<f32>,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = u.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    out.normal = in.normal;
    out.uv = in.uv;
    out.world_pos = in.position;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let tex = textureSample(t_diffuse, s_diffuse, in.uv);
    let ndotl = max(dot(normalize(in.normal), u.light_dir), 0.0);
    let light = u.ambient + (1.0 - u.ambient) * ndotl;
    let base = in.color * tex.rgb;
    // fog
    let dist = distance(in.world_pos, u.camera_pos);
    let fog_factor = clamp((60.0 - dist) / 60.0, 0.0, 1.0);
    let fog_color = vec3<f32>(0.53, 0.81, 0.92);
    let color = mix(fog_color, base * light, fog_factor);
    return vec4<f32>(color, 1.0);
}
"#;
