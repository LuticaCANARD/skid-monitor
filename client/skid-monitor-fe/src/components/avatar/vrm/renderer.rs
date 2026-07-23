use super::loader::{AlphaMode, CpuMaterial, CpuMorphDelta, CpuVrmScene, VrmVertex};
use super::runtime::{FrameInput, VrmRuntimeState};
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
    animation: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    alpha: [f32; 4],
    shade: [f32; 4],
    toon: [f32; 4],
    rim: [f32; 4],
    emissive_outline: [f32; 4],
    outline: [f32; 4],
    uv_animation: [f32; 4],
    matcap_normal: [f32; 4],
    texture_flags: [f32; 4],
    texture_flags_2: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DrawUniform {
    vertex_start: u32,
    vertex_count: u32,
    morph_count: u32,
    _padding: u32,
}

struct VrmCallback {
    scene: Arc<CpuVrmScene>,
    uniform: SceneUniform,
    time: f32,
    expression: String,
    crossfade_seconds: f32,
    look_yaw_degrees: f32,
    look_pitch_degrees: f32,
    spring_bone_enabled: bool,
    look_at_enabled: bool,
}

struct VrmClearCallback;

struct VrmRenderResources {
    pipeline_layout: wgpu::PipelineLayout,
    target_format: wgpu::TextureFormat,
    material_bind_group_layout: wgpu::BindGroupLayout,
    pose_bind_group_layout: wgpu::BindGroupLayout,
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
    reversed_front_face: bool,
    runtime: VrmRuntimeState,
    pipelines: VrmPipelines,
}

struct VrmPipelines {
    opaque: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    blend_depth_write: wgpu::RenderPipeline,
    outline: wgpu::RenderPipeline,
    outline_reversed: wgpu::RenderPipeline,
}

struct GpuDraw {
    source_draw_index: usize,
    vertices: Range<u32>,
    material_index: usize,
    sort_depth: f32,
    pose_bind_group: wgpu::BindGroup,
    pose_buffer: wgpu::Buffer,
    morph_target_indices: Vec<usize>,
    morph_weights_buffer: wgpu::Buffer,
    _morph_delta_buffer: wgpu::Buffer,
    _draw_uniform_buffer: wgpu::Buffer,
}

struct GpuMaterial {
    alpha_mode: AlphaMode,
    bind_group: wgpu::BindGroup,
    _uniform_buffer: wgpu::Buffer,
    has_outline: bool,
    transparent_with_z_write: bool,
    render_queue_offset: i32,
}

struct GpuTexture {
    _texture: wgpu::Texture,
    srgb_view: wgpu::TextureView,
    linear_view: wgpu::TextureView,
    _sampler: wgpu::Sampler,
}

pub(super) fn install(cc: &eframe::CreationContext<'_>) -> bool {
    let Some(render_state) = cc.wgpu_render_state.as_ref() else {
        return false;
    };
    // wgpu panics on uncaptured device errors by default. A validated custom shader or a
    // VRM model that Naga accepts but the active backend rejects at pipeline-creation time
    // must not take the whole dashboard down with it, so replace the fatal default with a
    // logged, non-fatal handler (RFC 0003 fallback policy).
    render_state
        .device
        .on_uncaptured_error(Arc::new(|error: wgpu::Error| {
            log::error!("VRM renderer WGPU error ignored to keep the dashboard running: {error}");
        }));
    let resources = VrmRenderResources::new(&render_state.device, render_state.target_format);
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(resources);
    true
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    painter: &egui::Painter,
    rect: egui::Rect,
    scene: Arc<CpuVrmScene>,
    time: f32,
    expression: &str,
    crossfade_seconds: f32,
    look_yaw_degrees: f32,
    look_pitch_degrees: f32,
    spring_bone_enabled: bool,
    look_at_enabled: bool,
) {
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
        animation: [time, 0.0, 0.0, 0.0],
    };
    painter.add(egui_wgpu::Callback::new_paint_callback(
        rect,
        VrmCallback {
            scene,
            uniform,
            time,
            expression: expression.to_string(),
            crossfade_seconds,
            look_yaw_degrees,
            look_pitch_degrees,
            spring_bone_enabled,
            look_at_enabled,
        },
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
        if let Some(scene) = resources.scene.as_mut() {
            let frame = self.scene.evaluate_frame(
                FrameInput {
                    time: self.time,
                    crossfade_seconds: self.crossfade_seconds,
                    expression: &self.expression,
                    look_yaw_degrees: self.look_yaw_degrees,
                    look_pitch_degrees: self.look_pitch_degrees,
                    spring_bone_enabled: self.spring_bone_enabled,
                    look_at_enabled: self.look_at_enabled,
                },
                &mut scene.runtime,
            );
            for draw in &scene.draws {
                let Some(matrices) = frame.pose_matrices.get(draw.source_draw_index) else {
                    continue;
                };
                queue.write_buffer(&draw.pose_buffer, 0, bytemuck::cast_slice(matrices));
                if let Some(cpu_draw) = self.scene.draws.get(draw.source_draw_index) {
                    let weights = draw
                        .morph_target_indices
                        .iter()
                        .map(|target| {
                            frame
                                .morph_weights
                                .get(&(cpu_draw.node_index, *target))
                                .copied()
                                .unwrap_or(0.0)
                        })
                        .collect::<Vec<_>>();
                    if !weights.is_empty() {
                        queue.write_buffer(
                            &draw.morph_weights_buffer,
                            0,
                            bytemuck::cast_slice(&weights),
                        );
                    }
                }
            }
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
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<MaterialUniform>() as u64,
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        let pose_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skid-vrm-pose-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(64),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<CpuMorphDelta>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(4),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<DrawUniform>() as u64
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
                Some(&pose_bind_group_layout),
            ],
            immediate_size: 0,
        });
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
            pipeline_layout,
            target_format,
            material_bind_group_layout,
            pose_bind_group_layout,
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
        let neutral_normal_texture_index = textures.len();
        textures.push(upload_texture(device, queue, 1, 1, &[128, 128, 255, 255]));
        let materials: Vec<GpuMaterial> = scene
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
                    &textures,
                    texture_index,
                    white_texture_index,
                    neutral_normal_texture_index,
                )
            })
            .collect();
        let pose_matrices = scene.pose_matrices(0.0);
        let mut draws = scene
            .draws
            .iter()
            .enumerate()
            .zip(pose_matrices)
            .map(|((source_draw_index, draw), matrices)| {
                let pose_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("skid-vrm-pose-matrices"),
                    contents: bytemuck::cast_slice(&matrices),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                });
                let morph_target_indices = draw
                    .morph_targets
                    .iter()
                    .map(|target| target.target_index)
                    .collect::<Vec<_>>();
                let morph_deltas = draw
                    .morph_targets
                    .iter()
                    .flat_map(|target| target.deltas.iter().copied())
                    .collect::<Vec<_>>();
                let morph_deltas = if morph_deltas.is_empty() {
                    vec![CpuMorphDelta::zeroed()]
                } else {
                    morph_deltas
                };
                let morph_delta_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("skid-vrm-morph-deltas"),
                        contents: bytemuck::cast_slice(&morph_deltas),
                        usage: wgpu::BufferUsages::STORAGE,
                    });
                let morph_weights = vec![0.0_f32; morph_target_indices.len().max(1)];
                let morph_weights_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("skid-vrm-morph-weights"),
                        contents: bytemuck::cast_slice(&morph_weights),
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    });
                let draw_uniform_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("skid-vrm-draw-uniform"),
                        contents: bytemuck::bytes_of(&DrawUniform {
                            vertex_start: draw.vertices.start,
                            vertex_count: draw.vertices.end - draw.vertices.start,
                            morph_count: morph_target_indices.len() as u32,
                            _padding: 0,
                        }),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let pose_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("skid-vrm-pose-bind-group"),
                    layout: &self.pose_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: pose_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: morph_delta_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: morph_weights_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: draw_uniform_buffer.as_entire_binding(),
                        },
                    ],
                });
                GpuDraw {
                    source_draw_index,
                    vertices: draw.vertices.clone(),
                    material_index: draw.material_index,
                    sort_depth: draw.center_z * scene.front_direction,
                    pose_bind_group,
                    pose_buffer,
                    morph_target_indices,
                    morph_weights_buffer,
                    _morph_delta_buffer: morph_delta_buffer,
                    _draw_uniform_buffer: draw_uniform_buffer,
                }
            })
            .collect::<Vec<_>>();
        draws.sort_by(|left, right| {
            let left_queue = materials
                .get(left.material_index)
                .map_or(0, |material| material.render_queue_offset);
            let right_queue = materials
                .get(right.material_index)
                .map_or(0, |material| material.render_queue_offset);
            left_queue
                .cmp(&right_queue)
                .then_with(|| left.sort_depth.total_cmp(&right.sort_depth))
        });
        let shader_source = scene
            .custom_shader_source
            .as_deref()
            .unwrap_or(include_str!("shader.wgsl"));
        let pipelines = VrmPipelines::new(
            device,
            &self.pipeline_layout,
            self.target_format,
            shader_source,
            scene.custom_shader_source.is_some(),
        );
        GpuScene {
            scene_id: scene.scene_id,
            vertex_buffer,
            draws,
            materials,
            _textures: textures,
            reversed_front_face: scene.front_direction < 0.0,
            runtime: VrmRuntimeState::default(),
            pipelines,
        }
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        let Some(scene) = &self.scene else {
            return;
        };
        render_pass.set_bind_group(0, &self.scene_bind_group, &[]);
        render_pass.set_vertex_buffer(0, scene.vertex_buffer.slice(..));

        render_pass.set_pipeline(if scene.reversed_front_face {
            &scene.pipelines.outline_reversed
        } else {
            &scene.pipelines.outline
        });
        for draw in &scene.draws {
            let Some(material) = scene.materials.get(draw.material_index) else {
                continue;
            };
            if !material.has_outline {
                continue;
            }
            render_pass.set_bind_group(1, &material.bind_group, &[]);
            render_pass.set_bind_group(2, &draw.pose_bind_group, &[]);
            render_pass.draw(draw.vertices.clone(), 0..1);
        }

        render_pass.set_pipeline(&scene.pipelines.opaque);
        for draw in &scene.draws {
            let Some(material) = scene.materials.get(draw.material_index) else {
                continue;
            };
            if material.alpha_mode == AlphaMode::Blend {
                continue;
            }
            render_pass.set_bind_group(1, &material.bind_group, &[]);
            render_pass.set_bind_group(2, &draw.pose_bind_group, &[]);
            render_pass.draw(draw.vertices.clone(), 0..1);
        }

        render_pass.set_pipeline(&scene.pipelines.blend_depth_write);
        for draw in &scene.draws {
            let Some(material) = scene.materials.get(draw.material_index) else {
                continue;
            };
            if material.alpha_mode != AlphaMode::Blend || !material.transparent_with_z_write {
                continue;
            }
            render_pass.set_bind_group(1, &material.bind_group, &[]);
            render_pass.set_bind_group(2, &draw.pose_bind_group, &[]);
            render_pass.draw(draw.vertices.clone(), 0..1);
        }

        render_pass.set_pipeline(&scene.pipelines.blend);
        for draw in &scene.draws {
            let Some(material) = scene.materials.get(draw.material_index) else {
                continue;
            };
            if material.alpha_mode != AlphaMode::Blend || material.transparent_with_z_write {
                continue;
            }
            render_pass.set_bind_group(1, &material.bind_group, &[]);
            render_pass.set_bind_group(2, &draw.pose_bind_group, &[]);
            render_pass.draw(draw.vertices.clone(), 0..1);
        }
    }
}

impl VrmPipelines {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        target_format: wgpu::TextureFormat,
        source: &str,
        custom: bool,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(if custom {
                "skid-vrm-custom-shader"
            } else {
                "skid-vrm-shader"
            }),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let fragment_entry = if target_format.is_srgb() {
            "fs_main_linear_framebuffer"
        } else {
            "fs_main_gamma_framebuffer"
        };
        let outline_fragment_entry = if target_format.is_srgb() {
            "fs_outline_linear_framebuffer"
        } else {
            "fs_outline_gamma_framebuffer"
        };
        Self {
            opaque: create_pipeline(
                device,
                layout,
                &shader,
                target_format,
                PipelineConfig {
                    vertex_entry: "vs_main",
                    fragment_entry,
                    blend: None,
                    depth_write_enabled: true,
                    cull_mode: None,
                    label: "skid-vrm-opaque-pipeline",
                },
            ),
            blend: create_pipeline(
                device,
                layout,
                &shader,
                target_format,
                PipelineConfig {
                    vertex_entry: "vs_main",
                    fragment_entry,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    depth_write_enabled: false,
                    cull_mode: None,
                    label: "skid-vrm-blend-pipeline",
                },
            ),
            blend_depth_write: create_pipeline(
                device,
                layout,
                &shader,
                target_format,
                PipelineConfig {
                    vertex_entry: "vs_main",
                    fragment_entry,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    depth_write_enabled: true,
                    cull_mode: None,
                    label: "skid-vrm-blend-depth-write-pipeline",
                },
            ),
            outline: create_pipeline(
                device,
                layout,
                &shader,
                target_format,
                PipelineConfig {
                    vertex_entry: "vs_outline",
                    fragment_entry: outline_fragment_entry,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    depth_write_enabled: true,
                    cull_mode: Some(wgpu::Face::Front),
                    label: "skid-vrm-outline-pipeline",
                },
            ),
            outline_reversed: create_pipeline(
                device,
                layout,
                &shader,
                target_format,
                PipelineConfig {
                    vertex_entry: "vs_outline",
                    fragment_entry: outline_fragment_entry,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    depth_write_enabled: true,
                    cull_mode: Some(wgpu::Face::Back),
                    label: "skid-vrm-outline-reversed-pipeline",
                },
            ),
        }
    }
}

struct PipelineConfig {
    vertex_entry: &'static str,
    fragment_entry: &'static str,
    blend: Option<wgpu::BlendState>,
    depth_write_enabled: bool,
    cull_mode: Option<wgpu::Face>,
    label: &'static str,
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    config: PipelineConfig,
) -> wgpu::RenderPipeline {
    const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x2,
        3 => Float32x4,
        4 => Uint16x4,
        5 => Float32x4
    ];
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
            entry_point: Some(config.vertex_entry),
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
            cull_mode: config.cull_mode,
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
    textures: &[GpuTexture],
    base_texture_index: usize,
    white_texture_index: usize,
    neutral_normal_texture_index: usize,
) -> GpuMaterial {
    let texture = |index: Option<usize>, fallback: usize| {
        &textures[index
            .filter(|index| *index < white_texture_index)
            .unwrap_or(fallback)]
    };
    let base_texture = &textures[base_texture_index];
    let shade_texture = texture(material.mtoon.shade_texture, white_texture_index);
    let normal_texture = texture(material.mtoon.normal_texture, neutral_normal_texture_index);
    let matcap_texture = texture(material.mtoon.matcap_texture, white_texture_index);
    let rim_texture = texture(material.mtoon.rim_texture, white_texture_index);
    let outline_width_texture = texture(material.mtoon.outline_width_texture, white_texture_index);
    let uniform = MaterialUniform {
        base_color: material.base_color,
        alpha: [
            material.alpha_cutoff,
            if material.alpha_mode == AlphaMode::Mask {
                1.0
            } else {
                0.0
            },
            if material.mtoon.enabled { 1.0 } else { 0.0 },
            0.0,
        ],
        shade: [
            material.mtoon.shade_color[0],
            material.mtoon.shade_color[1],
            material.mtoon.shade_color[2],
            material.mtoon.shading_shift,
        ],
        toon: [
            material.mtoon.shading_toony,
            material.mtoon.gi_equalization,
            material.mtoon.rim_fresnel_power,
            material.mtoon.rim_lift,
        ],
        rim: [
            material.mtoon.parametric_rim_color[0],
            material.mtoon.parametric_rim_color[1],
            material.mtoon.parametric_rim_color[2],
            0.0,
        ],
        emissive_outline: [
            material.mtoon.emissive_color[0],
            material.mtoon.emissive_color[1],
            material.mtoon.emissive_color[2],
            material.mtoon.outline_width,
        ],
        outline: [
            material.mtoon.outline_color[0],
            material.mtoon.outline_color[1],
            material.mtoon.outline_color[2],
            material.mtoon.outline_lighting_mix,
        ],
        uv_animation: [
            material.mtoon.uv_scroll[0],
            material.mtoon.uv_scroll[1],
            material.mtoon.uv_rotation,
            0.0,
        ],
        matcap_normal: [
            material.mtoon.matcap_color[0],
            material.mtoon.matcap_color[1],
            material.mtoon.matcap_color[2],
            material.mtoon.normal_scale,
        ],
        texture_flags: [
            f32::from(material.mtoon.shade_texture.is_some()),
            f32::from(material.mtoon.normal_texture.is_some()),
            f32::from(material.mtoon.matcap_texture.is_some()),
            f32::from(material.mtoon.rim_texture.is_some()),
        ],
        texture_flags_2: [
            f32::from(material.mtoon.outline_width_texture.is_some()),
            material.mtoon.rim_lighting_mix,
            f32::from(material.mtoon.outline_width_texture_uses_red),
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
                resource: wgpu::BindingResource::TextureView(&base_texture.srgb_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&base_texture._sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&shade_texture.srgb_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&normal_texture.linear_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&matcap_texture.srgb_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&rim_texture.srgb_view),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&outline_width_texture.linear_view),
            },
        ],
    });
    GpuMaterial {
        alpha_mode: material.alpha_mode,
        bind_group,
        _uniform_buffer: uniform_buffer,
        has_outline: material.mtoon.enabled && material.mtoon.outline_width > f32::EPSILON,
        transparent_with_z_write: material.mtoon.transparent_with_z_write,
        render_queue_offset: material.mtoon.render_queue_offset,
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
        label: Some("skid-vrm-texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
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
    let linear_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let srgb_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("skid-vrm-texture-srgb-view"),
        format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        ..Default::default()
    });
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
        srgb_view,
        linear_view,
        _sampler: sampler,
    }
}

#[cfg(test)]
mod tests {
    use super::super::loader::CpuMtoonMaterial;
    use super::*;

    #[test]
    fn vrm_shader_parses_and_validates() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("shader.wgsl"))
            .expect("parse VRM WGSL shader");
        let mut validator = wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        );

        validator
            .validate(&module)
            .expect("validate VRM WGSL shader");
    }

    #[tokio::test]
    async fn vrm_pipeline_and_dedicated_texture_bindings_build_on_an_available_gpu() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
        else {
            return;
        };
        eprintln!("VRM WGPU validation adapter: {:?}", adapter.get_info());
        let Ok((device, queue)) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
        else {
            return;
        };
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let resources = VrmRenderResources::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb);
        let _default_pipelines = VrmPipelines::new(
            &device,
            &resources.pipeline_layout,
            resources.target_format,
            include_str!("shader.wgsl"),
            false,
        );
        let custom_source = super::super::custom_shader::compose(include_str!(
            "../../../../examples/custom-material.wgsl"
        ))
        .expect("compose example custom material shader");
        let _custom_pipelines = VrmPipelines::new(
            &device,
            &resources.pipeline_layout,
            resources.target_format,
            &custom_source,
            true,
        );
        let textures = vec![
            upload_texture(&device, &queue, 1, 1, &[255, 255, 255, 255]),
            upload_texture(&device, &queue, 1, 1, &[255, 255, 255, 255]),
            upload_texture(&device, &queue, 1, 1, &[128, 128, 255, 255]),
        ];
        let material = CpuMaterial {
            base_color: [1.0; 4],
            texture_index: Some(0),
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            mtoon: CpuMtoonMaterial {
                enabled: true,
                shade_texture: Some(0),
                normal_texture: Some(0),
                matcap_texture: Some(0),
                rim_texture: Some(0),
                outline_width_texture: Some(0),
                ..Default::default()
            },
        };
        let _material = upload_material(
            &device,
            &resources.material_bind_group_layout,
            &material,
            &textures,
            0,
            1,
            2,
        );

        let error = error_scope.pop().await;
        assert!(error.is_none(), "WGPU rejected VRM resources: {error:?}");
    }
}
