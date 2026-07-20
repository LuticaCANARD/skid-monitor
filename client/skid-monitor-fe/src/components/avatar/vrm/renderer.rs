use super::loader::{AlphaMode, CpuMaterial, CpuVrmScene, VrmVertex};
use bytemuck::{Pod, Zeroable};
use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};
use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::Arc;
use wgpu::util::DeviceExt as _;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneUniform {
    center_and_scale: [f32; 4],
    projection: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    alpha: [f32; 4],
}

struct VrmCallback {
    scene: Arc<CpuVrmScene>,
    uniform: SceneUniform,
}

struct VrmClearCallback;

struct VrmRenderResources {
    opaque_pipeline: wgpu::RenderPipeline,
    blend_pipeline: wgpu::RenderPipeline,
    material_bind_group_layout: wgpu::BindGroupLayout,
    scene_uniform_buffer: wgpu::Buffer,
    scene_bind_group: wgpu::BindGroup,
    scene: Option<GpuScene>,
}

struct GpuScene {
    scene_id: u64,
    vertex_buffer: wgpu::Buffer,
    draws: Vec<GpuDraw>,
    materials: Vec<GpuMaterial>,
    _textures: Vec<GpuTexture>,
}

struct GpuDraw {
    vertices: Range<u32>,
    material_index: usize,
    sort_depth: f32,
}

struct GpuMaterial {
    alpha_mode: AlphaMode,
    bind_group: wgpu::BindGroup,
    _uniform_buffer: wgpu::Buffer,
}

struct GpuTexture {
    _texture: wgpu::Texture,
    _texture_view: wgpu::TextureView,
    _sampler: wgpu::Sampler,
}

pub(super) fn install(cc: &eframe::CreationContext<'_>) -> bool {
    let Some(render_state) = cc.wgpu_render_state.as_ref() else {
        return false;
    };
    let resources = VrmRenderResources::new(&render_state.device, render_state.target_format);
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(resources);
    true
}

pub(super) fn paint(painter: &egui::Painter, rect: egui::Rect, scene: Arc<CpuVrmScene>) {
    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        return;
    }
    let aspect = (rect.width() / rect.height()).max(0.01);
    let width = scene.extent[0].abs().max(0.001);
    let height = scene.extent[1].abs().max(0.001);
    let scale = (1.8 / height).min(1.8 * aspect / width);
    let depth_span = scene.extent[2].abs().max(0.001);
    let z_max = scene.center[2] * scene.front_direction + depth_span * 0.5;
    let uniform = SceneUniform {
        center_and_scale: [scene.center[0], scene.center[1], scene.center[2], scale],
        projection: [aspect, z_max, depth_span, scene.front_direction],
    };
    painter.add(egui_wgpu::Callback::new_paint_callback(
        rect,
        VrmCallback { scene, uniform },
    ));
}

pub(super) fn clear(painter: &egui::Painter, rect: egui::Rect) {
    painter.add(egui_wgpu::Callback::new_paint_callback(
        rect,
        VrmClearCallback,
    ));
}

impl egui_wgpu::CallbackTrait for VrmClearCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(resources) = callback_resources.get_mut::<VrmRenderResources>() {
            resources.scene = None;
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        _render_pass: &mut wgpu::RenderPass<'static>,
        _callback_resources: &egui_wgpu::CallbackResources,
    ) {
    }
}

impl egui_wgpu::CallbackTrait for VrmCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = callback_resources.get_mut::<VrmRenderResources>() else {
            return Vec::new();
        };
        if resources.scene.as_ref().map(|scene| scene.scene_id) != Some(self.scene.scene_id) {
            resources.scene = Some(resources.upload_scene(device, queue, &self.scene));
        }
        queue.write_buffer(
            &resources.scene_uniform_buffer,
            0,
            bytemuck::bytes_of(&self.uniform),
        );
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<VrmRenderResources>() else {
            return;
        };
        resources.paint(render_pass);
    }
}

impl VrmRenderResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let scene_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skid-vrm-scene-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<SceneUniform>() as u64
                        ),
                    },
                    count: None,
                }],
            });
        let material_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skid-vrm-material-layout"),
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<MaterialUniform>() as u64,
                            ),
                        },
                        count: None,
                    },
                ],
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skid-vrm-pipeline-layout"),
            bind_group_layouts: &[
                Some(&scene_bind_group_layout),
                Some(&material_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skid-vrm-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let fragment_entry = if target_format.is_srgb() {
            "fs_main_linear_framebuffer"
        } else {
            "fs_main_gamma_framebuffer"
        };
        let opaque_pipeline = create_pipeline(
            device,
            &pipeline_layout,
            &shader,
            target_format,
            PipelineConfig {
                fragment_entry,
                blend: None,
                depth_write_enabled: true,
                label: "skid-vrm-opaque-pipeline",
            },
        );
        let blend_pipeline = create_pipeline(
            device,
            &pipeline_layout,
            &shader,
            target_format,
            PipelineConfig {
                fragment_entry,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                depth_write_enabled: false,
                label: "skid-vrm-blend-pipeline",
            },
        );
        let scene_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("skid-vrm-scene-uniform"),
            contents: bytemuck::bytes_of(&SceneUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skid-vrm-scene-bind-group"),
            layout: &scene_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            opaque_pipeline,
            blend_pipeline,
            material_bind_group_layout,
            scene_uniform_buffer,
            scene_bind_group,
            scene: None,
        }
    }

    fn upload_scene(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &CpuVrmScene,
    ) -> GpuScene {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("skid-vrm-vertices"),
            contents: bytemuck::cast_slice(&scene.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mut textures = scene
            .textures
            .iter()
            .map(|texture| {
                upload_texture(device, queue, texture.width, texture.height, &texture.rgba)
            })
            .collect::<Vec<_>>();
        let white_texture_index = textures.len();
        textures.push(upload_texture(device, queue, 1, 1, &[255, 255, 255, 255]));
        let materials = scene
            .materials
            .iter()
            .map(|material| {
                let texture_index = material
                    .texture_index
                    .filter(|index| *index < white_texture_index)
                    .unwrap_or(white_texture_index);
                upload_material(
                    device,
                    &self.material_bind_group_layout,
                    material,
                    &textures[texture_index],
                )
            })
            .collect();
        let mut draws = scene
            .draws
            .iter()
            .map(|draw| GpuDraw {
                vertices: draw.vertices.clone(),
                material_index: draw.material_index,
                sort_depth: draw.center_z * scene.front_direction,
            })
            .collect::<Vec<_>>();
        draws.sort_by(|left, right| left.sort_depth.total_cmp(&right.sort_depth));
        GpuScene {
            scene_id: scene.scene_id,
            vertex_buffer,
            draws,
            materials,
            _textures: textures,
        }
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        let Some(scene) = &self.scene else {
            return;
        };
        render_pass.set_bind_group(0, &self.scene_bind_group, &[]);
        render_pass.set_vertex_buffer(0, scene.vertex_buffer.slice(..));

        render_pass.set_pipeline(&self.opaque_pipeline);
        for draw in &scene.draws {
            let Some(material) = scene.materials.get(draw.material_index) else {
                continue;
            };
            if material.alpha_mode == AlphaMode::Blend {
                continue;
            }
            render_pass.set_bind_group(1, &material.bind_group, &[]);
            render_pass.draw(draw.vertices.clone(), 0..1);
        }

        render_pass.set_pipeline(&self.blend_pipeline);
        for draw in &scene.draws {
            let Some(material) = scene.materials.get(draw.material_index) else {
                continue;
            };
            if material.alpha_mode != AlphaMode::Blend {
                continue;
            }
            render_pass.set_bind_group(1, &material.bind_group, &[]);
            render_pass.draw(draw.vertices.clone(), 0..1);
        }
    }
}

struct PipelineConfig {
    fragment_entry: &'static str,
    blend: Option<wgpu::BlendState>,
    depth_write_enabled: bool,
    label: &'static str,
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    config: PipelineConfig,
) -> wgpu::RenderPipeline {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
    let vertex_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<VrmVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(config.label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[vertex_layout],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(config.fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: config.blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(config.depth_write_enabled),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn upload_material(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    material: &CpuMaterial,
    texture: &GpuTexture,
) -> GpuMaterial {
    let uniform = MaterialUniform {
        base_color: material.base_color,
        alpha: [
            material.alpha_cutoff,
            if material.alpha_mode == AlphaMode::Mask {
                1.0
            } else {
                0.0
            },
            0.0,
            0.0,
        ],
    };
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("skid-vrm-material-uniform"),
        contents: bytemuck::bytes_of(&uniform),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("skid-vrm-material-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture._texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&texture._sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform_buffer.as_entire_binding(),
            },
        ],
    });
    GpuMaterial {
        alpha_mode: material.alpha_mode,
        bind_group,
        _uniform_buffer: uniform_buffer,
    }
}

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> GpuTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("skid-vrm-base-color-texture"),
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
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("skid-vrm-base-color-sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    GpuTexture {
        _texture: texture,
        _texture_view: texture_view,
        _sampler: sampler,
    }
}
