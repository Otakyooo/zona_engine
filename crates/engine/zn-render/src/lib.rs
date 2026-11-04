//! zn-render: рендерер на wgpu (Vulkan primary, DX12/DX11 fallback).

pub mod geom;

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
    /// Границы в локальном пространстве меша. Нужны для отсечения по пирамиде
    /// видимости: без них приходилось рисовать всё, что попало в снапшот.
    pub bounds: MeshBounds,
}

/// Ось-выровненные границы меша.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl MeshBounds {
    pub fn from_vertices(vertices: &[Vertex]) -> Self {
        if vertices.is_empty() {
            return Self {
                min: Vec3::ZERO,
                max: Vec3::ZERO,
            };
        }
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for v in vertices {
            let p = Vec3::from(v.position);
            min = min.min(p);
            max = max.max(p);
        }
        Self { min, max }
    }

    /// Границы, повёрнутые и сдвинутые матрицей модели. Берётся AABB восьми
    /// углов: точнее для отсечения не нужно, а стоимость копеечная.
    pub fn transformed(&self, model: Mat4) -> (Vec3, Vec3) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for i in 0..8u32 {
            let corner = Vec3::new(
                if i & 1 == 0 { self.min.x } else { self.max.x },
                if i & 2 == 0 { self.min.y } else { self.max.y },
                if i & 4 == 0 { self.min.z } else { self.max.z },
            );
            let p = model.transform_point3(corner);
            min = min.min(p);
            max = max.max(p);
        }
        (min, max)
    }
}

/// Пирамида видимости: шесть плоскостей, извлечённых из `view_proj`.
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    planes: [[f32; 4]; 6],
}

impl Frustum {
    pub fn from_view_proj(m: Mat4) -> Self {
        // glam хранит матрицу по столбцам; плоскости собираются из строк.
        let r0 = [m.x_axis.x, m.y_axis.x, m.z_axis.x, m.w_axis.x];
        let r1 = [m.x_axis.y, m.y_axis.y, m.z_axis.y, m.w_axis.y];
        let r2 = [m.x_axis.z, m.y_axis.z, m.z_axis.z, m.w_axis.z];
        let r3 = [m.x_axis.w, m.y_axis.w, m.z_axis.w, m.w_axis.w];
        let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
        let sub = |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
        let mut planes = [
            add(r3, r0), // left
            sub(r3, r0), // right
            add(r3, r1), // bottom
            sub(r3, r1), // top
            r2,          // near: clip z >= 0
            sub(r3, r2), // far
        ];
        for p in &mut planes {
            let n = Vec3::new(p[0], p[1], p[2]).length();
            if n > 0.0 {
                for v in p.iter_mut() {
                    *v /= n;
                }
            }
        }
        Self { planes }
    }

    /// Пересекается ли AABB с пирамидой. Ложные срабатывания допустимы
    /// (объект лишний раз нарисуется), пропуски — нет.
    pub fn intersects_aabb(&self, min: Vec3, max: Vec3) -> bool {
        for p in &self.planes {
            let x = if p[0] >= 0.0 { max.x } else { min.x };
            let y = if p[1] >= 0.0 { max.y } else { min.y };
            let z = if p[2] >= 0.0 { max.z } else { min.z };
            if p[0] * x + p[1] * y + p[2] * z + p[3] < 0.0 {
                return false;
            }
        }
        true
    }
}

/// Что произошло в кадре: сколько объектов прошло отсечение, сколько
/// нарисовано и сколько draw call'ов отправлено.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub total: usize,
    pub drawn: usize,
    pub culled: usize,
    pub draw_calls: usize,
}

pub struct Texture {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
}

/// Что рисовать в кадре: меш, текстура и матрица модели.
///
/// Статическая геометрия приходит с `Mat4::IDENTITY` — её вершины уже в мире.
/// Подвижные сущности строятся в локальном пространстве, поэтому их трансформ
/// передаётся здесь, а не запекается в вершины: иначе меш пришлось бы
/// пересоздавать при каждом движении.
pub struct RenderItem<'a> {
    pub mesh: &'a Mesh,
    pub texture: Option<&'a Texture>,
    pub model: Mat4,
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

/// Размер матрицы модели в байтах (4×4 f32).
const MODEL_UNIFORM_SIZE: u64 = 64;
/// Сколько объектов можно нарисовать за кадр. Слот занимает выравнивание
/// адаптера (обычно 256 байт), поэтому потолок держим скромным.
const MAX_RENDER_OBJECTS: usize = 512;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModelUniform {
    model: [[f32; 4]; 4],
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
    model_buffer: wgpu::Buffer,
    model_bind_group: wgpu::BindGroup,
    /// Шаг между слотами матриц: выравнивание адаптера, но не меньше 64 байт.
    model_stride: u64,
    default_texture: Texture,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    egui_renderer: egui_wgpu::Renderer,
    egui_format: wgpu::TextureFormat,
    stats: RenderStats,
    overflow_warned: bool,
}

/// Кадр интерфейса, подготовленный клиентом.
///
/// `FullOutput` передаётся по владению: `tessellate()` забирает `shapes`.
pub struct UiFrame<'a> {
    pub ctx: &'a egui::Context,
    pub output: egui::FullOutput,
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
            // egui пишет цвета в гамма-пространстве и на sRGB-таргете получает
            // двойное преобразование. Держим рядом линейный view-формат: сцена
            // рисуется в sRGB-вью, HUD — в линейный.
            view_formats: if format.is_srgb() {
                vec![format.remove_srgb_suffix()]
            } else {
                vec![]
            },
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

        // Матрица модели для подвижных сущностей: слот на объект, смещение
        // выставляется динамически.
        let model_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(MODEL_UNIFORM_SIZE),
                },
                count: None,
            }],
        });

        // Шаг слота — требование wgpu: смещение динамического offset должно
        // быть кратно выравниванию адаптера (обычно 256 байт).
        let model_stride =
            (device.limits().min_uniform_buffer_offset_alignment as u64).max(MODEL_UNIFORM_SIZE);
        let model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("model matrices"),
            size: model_stride * MAX_RENDER_OBJECTS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let model_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("model bg"),
            layout: &model_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &model_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(MODEL_UNIFORM_SIZE),
                }),
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
            bind_group_layouts: &[&uniform_bgl, &texture_bgl, &model_bgl],
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
        // HUD рисуется вторым проходом поверх сцены, без depth-аттачмента.
        // Формат — линейный (не sRGB), иначе egui применяет гамму дважды.
        let egui_format = config.format.remove_srgb_suffix();
        let egui_renderer = egui_wgpu::Renderer::new(&device, egui_format, None, 1, false);

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
            model_buffer,
            model_bind_group,
            model_stride,
            default_texture,
            depth_texture,
            depth_view,
            egui_renderer,
            egui_format,
            stats: RenderStats::default(),
            overflow_warned: false,
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
            bounds: MeshBounds::from_vertices(vertices),
        }
    }

    /// Статистика последнего кадра — для perf-лога, а не для геймплея.
    pub fn stats(&self) -> RenderStats {
        self.stats
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
        items: &[RenderItem<'_>],
        camera: &Camera,
        ui: Option<UiFrame<'_>>,
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

        // Отсечение по пирамиде видимости. Раньше в кадр шло всё, что попало
        // в снапшот, включая объекты за спиной; на большой карте это первое,
        // что упирается в бюджет кадра.
        let frustum = Frustum::from_view_proj(camera.view_proj());
        let mut visible: Vec<usize> = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let (min, max) = item.mesh.bounds.transformed(item.model);
            if frustum.intersects_aabb(min, max) {
                visible.push(i);
            }
        }
        let culled = items.len() - visible.len();

        // Один материал подряд: смена bind group между каждым объектом — лишняя
        // работа для драйвера даже при том же числе draw call'ов.
        visible.sort_by_key(|&i| {
            items[i]
                .texture
                .map(|t| t as *const Texture as usize)
                .unwrap_or(0)
        });

        let count = visible.len().min(MAX_RENDER_OBJECTS);
        if visible.len() > MAX_RENDER_OBJECTS {
            if !self.overflow_warned {
                log::warn!(
                    "кадр обрезан: {} видимых объектов, лимит {MAX_RENDER_OBJECTS}; \
                     поднимайте слоты вместе с картой",
                    visible.len()
                );
                self.overflow_warned = true;
            }
        } else {
            self.overflow_warned = false;
        }

        // Все матрицы кадра — одной записью. Слоты идут с шагом выравнивания,
        // поэтому хвост каждого слота остаётся нулевым, и это нормально.
        if count > 0 {
            let stride = self.model_stride as usize;
            let mut data = vec![0u8; stride * count];
            for (slot, &i) in visible.iter().take(count).enumerate() {
                let m = ModelUniform {
                    model: items[i].model.to_cols_array_2d(),
                };
                let start = slot * stride;
                data[start..start + MODEL_UNIFORM_SIZE as usize]
                    .copy_from_slice(bytemuck::bytes_of(&m));
            }
            self.queue.write_buffer(&self.model_buffer, 0, &data);
        }

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
            for (slot, &i) in visible.iter().take(count).enumerate() {
                let item = &items[i];
                let t = item.texture.unwrap_or(&self.default_texture);
                pass.set_bind_group(1, &t.bind_group, &[]);
                let offset = (slot as u64 * self.model_stride) as u32;
                pass.set_bind_group(2, &self.model_bind_group, &[offset]);
                pass.set_vertex_buffer(0, item.mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(item.mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..item.mesh.num_indices, 0, 0..1);
            }
        }
        // Второй проход: HUD поверх сцены. LoadOp::Load обязателен, иначе
        // интерфейс затрёт кадр; depth не нужен — HUD плоский.
        let mut extra_buffers: Vec<wgpu::CommandBuffer> = Vec::new();
        if let Some(ui) = ui {
            let ppp = ui.output.pixels_per_point.clamp(0.1, 8.0);
            let jobs = ui.ctx.tessellate(ui.output.shapes, ppp);
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [self.config.width, self.config.height],
                pixels_per_point: ppp,
            };

            for (id, delta) in &ui.output.textures_delta.set {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
            extra_buffers = self.egui_renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &jobs,
                &screen,
            );

            {
                // Отдельный линейный вью того же таргета: сцена уже отрисована
                // через sRGB-вью, а egui ждёт не-sRGB.
                let hud_view = output.texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(self.egui_format),
                    ..Default::default()
                });
                let desc = wgpu::RenderPassDescriptor {
                    label: Some("hud pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &hud_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                };
                // forget_lifetime отвязывает проход от энкодера; после этого
                // любое обращение к encoder — ошибка рантайма, поэтому pass
                // живёт строго внутри этого блока.
                let mut pass = encoder.begin_render_pass(&desc).forget_lifetime();
                self.egui_renderer.render(&mut pass, &jobs, &screen);
            }

            for id in &ui.output.textures_delta.free {
                self.egui_renderer.free_texture(id);
            }
        }

        self.queue.submit(
            extra_buffers
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );
        output.present();
        self.stats = RenderStats {
            total: items.len(),
            drawn: count,
            culled,
            draw_calls: count,
        };
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
// Матрица модели приходит слотом из общего буфера: трансформ живёт в uniform,
// а не в вершинах, поэтому меш переиспользуется всеми экземплярами.
@group(2) @binding(0) var<uniform> model: mat4x4<f32>;

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
    let world = model * vec4<f32>(in.position, 1.0);
    out.clip_position = u.view_proj * world;
    out.color = in.color;
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    out.world_pos = world.xyz;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            eye: Vec3::new(0.0, 1.0, 5.0),
            target: Vec3::new(0.0, 1.0, 0.0),
            up: Vec3::Y,
            fov_y: 1.0,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        }
    }

    #[test]
    fn mesh_bounds_identity_is_source_bounds() {
        let b = MeshBounds {
            min: Vec3::new(-1.0, 0.0, -2.0),
            max: Vec3::new(1.0, 2.0, 2.0),
        };
        let (min, max) = b.transformed(Mat4::IDENTITY);
        assert_eq!(min, b.min);
        assert_eq!(max, b.max);
    }

    #[test]
    fn mesh_bounds_follow_translation() {
        let b = MeshBounds {
            min: Vec3::splat(-1.0),
            max: Vec3::splat(1.0),
        };
        let (min, max) = b.transformed(Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)));
        assert!((min.x - 9.0).abs() < 1e-5, "min={min:?}");
        assert!((max.x - 11.0).abs() < 1e-5, "max={max:?}");
    }

    #[test]
    fn frustum_keeps_object_in_front() {
        let f = Frustum::from_view_proj(camera().view_proj());
        assert!(
            f.intersects_aabb(Vec3::splat(-0.5), Vec3::splat(0.5)),
            "объект перед камерой должен проходить отсечение"
        );
    }

    #[test]
    fn frustum_rejects_object_behind_camera() {
        let f = Frustum::from_view_proj(camera().view_proj());
        // Камера смотрит в -Z; объект за спиной — на +Z от неё.
        assert!(!f.intersects_aabb(Vec3::new(-1.0, 0.0, 9.0), Vec3::new(1.0, 2.0, 11.0)));
    }

    #[test]
    fn frustum_rejects_far_and_sideways_objects() {
        let f = Frustum::from_view_proj(camera().view_proj());
        assert!(!f.intersects_aabb(Vec3::new(-1.0, 0.0, -300.0), Vec3::new(1.0, 2.0, -200.0)));
        assert!(!f.intersects_aabb(Vec3::new(40.0, 0.0, -1.0), Vec3::new(42.0, 2.0, 1.0)));
    }
}
