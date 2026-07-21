use super::animation::{CpuAnimationClip, NodeTransform, RetargetTarget};
use super::runtime::{
    ColliderShape, ConstraintKind, CpuCollider, CpuExpression, CpuExpressionSet, CpuLookAt,
    CpuNodeConstraint, CpuSpring, CpuSpringJoint, ExpressionOverride, FrameInput, LookAtKind,
    MorphBind, RangeMap, RuntimeRig, VrmRuntimeState,
};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use gltf::{
    Gltf, Semantic,
    accessor::{DataType, Dimensions},
    buffer, image as gltf_image,
    mesh::Mode,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::ops::Range;

const MAX_VRM_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_NODES: usize = 4096;
const MAX_NODE_DEPTH: usize = 128;
const MAX_PRIMITIVES: usize = 4096;
const MAX_TRIANGLES: usize = 300_000;
const MAX_SOURCE_VERTICES: usize = 1_000_000;
const MAX_OUTPUT_VERTICES: usize = MAX_TRIANGLES * 3;
const MAX_MORPH_DELTAS: usize = 2_000_000;
const MAX_SKIN_JOINTS: usize = 256;
const MAX_TEXTURES: usize = 64;
const MAX_TEXTURE_DIMENSION: u32 = 4096;
const MAX_TEXTURE_DECODE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_VRMA_FILE_BYTES: u64 = 64 * 1024 * 1024;

const REQUIRED_VRM_ONE_HUMANOID_BONES: [&str; 15] = [
    "hips",
    "spine",
    "head",
    "leftUpperLeg",
    "leftLowerLeg",
    "leftFoot",
    "rightUpperLeg",
    "rightLowerLeg",
    "rightFoot",
    "leftUpperArm",
    "leftLowerArm",
    "leftHand",
    "rightUpperArm",
    "rightLowerArm",
    "rightHand",
];

const REQUIRED_VRM_ZERO_HUMANOID_BONES: [&str; 17] = [
    "hips",
    "spine",
    "chest",
    "neck",
    "head",
    "leftUpperLeg",
    "leftLowerLeg",
    "leftFoot",
    "rightUpperLeg",
    "rightLowerLeg",
    "rightFoot",
    "leftUpperArm",
    "leftLowerArm",
    "leftHand",
    "rightUpperArm",
    "rightLowerArm",
    "rightHand",
];

const REQUIRED_VRM_ONE_HIERARCHY: [(&str, &str); 14] = [
    ("hips", "spine"),
    ("spine", "head"),
    ("hips", "leftUpperLeg"),
    ("leftUpperLeg", "leftLowerLeg"),
    ("leftLowerLeg", "leftFoot"),
    ("hips", "rightUpperLeg"),
    ("rightUpperLeg", "rightLowerLeg"),
    ("rightLowerLeg", "rightFoot"),
    ("spine", "leftUpperArm"),
    ("leftUpperArm", "leftLowerArm"),
    ("leftLowerArm", "leftHand"),
    ("spine", "rightUpperArm"),
    ("rightUpperArm", "rightLowerArm"),
    ("rightLowerArm", "rightHand"),
];

const REQUIRED_VRM_ZERO_HIERARCHY: [(&str, &str); 16] = [
    ("hips", "spine"),
    ("spine", "chest"),
    ("chest", "neck"),
    ("neck", "head"),
    ("hips", "leftUpperLeg"),
    ("leftUpperLeg", "leftLowerLeg"),
    ("leftLowerLeg", "leftFoot"),
    ("hips", "rightUpperLeg"),
    ("rightUpperLeg", "rightLowerLeg"),
    ("rightLowerLeg", "rightFoot"),
    ("chest", "leftUpperArm"),
    ("leftUpperArm", "leftLowerArm"),
    ("leftLowerArm", "leftHand"),
    ("chest", "rightUpperArm"),
    ("rightUpperArm", "rightLowerArm"),
    ("rightLowerArm", "rightHand"),
];

const SUPPORTED_REQUIRED_EXTENSIONS: [&str; 6] = [
    "VRMC_vrm",
    "VRM",
    "VRMC_materials_mtoon",
    "VRMC_springBone",
    "VRMC_node_constraint",
    "KHR_materials_unlit",
];
const SUPPORTED_VRMA_REQUIRED_EXTENSIONS: [&str; 1] = ["VRMC_vrm_animation"];

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(super) struct VrmVertex {
    pub(super) position: [f32; 3],
    pub(super) normal: [f32; 3],
    pub(super) uv: [f32; 2],
    pub(super) tangent: [f32; 4],
    pub(super) joints: [u16; 4],
    pub(super) weights: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(super) struct CpuMorphDelta {
    pub(super) position: [f32; 4],
    pub(super) normal: [f32; 4],
    pub(super) tangent: [f32; 4],
}

#[derive(Clone, Debug)]
pub(super) struct CpuMorphTarget {
    pub(super) target_index: usize,
    pub(super) deltas: Vec<CpuMorphDelta>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}

#[derive(Clone, Debug)]
pub(super) struct CpuTexture {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct CpuMaterial {
    pub(super) base_color: [f32; 4],
    pub(super) texture_index: Option<usize>,
    pub(super) alpha_mode: AlphaMode,
    pub(super) alpha_cutoff: f32,
    pub(super) mtoon: CpuMtoonMaterial,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CpuMtoonMaterial {
    pub(super) enabled: bool,
    pub(super) shade_color: [f32; 3],
    pub(super) shading_shift: f32,
    pub(super) shading_toony: f32,
    pub(super) gi_equalization: f32,
    pub(super) parametric_rim_color: [f32; 3],
    pub(super) rim_fresnel_power: f32,
    pub(super) rim_lift: f32,
    pub(super) emissive_color: [f32; 3],
    pub(super) outline_width: f32,
    pub(super) outline_color: [f32; 3],
    pub(super) outline_lighting_mix: f32,
    pub(super) transparent_with_z_write: bool,
    pub(super) render_queue_offset: i32,
    pub(super) uv_scroll: [f32; 2],
    pub(super) uv_rotation: f32,
    pub(super) shade_texture: Option<usize>,
    pub(super) normal_texture: Option<usize>,
    pub(super) matcap_texture: Option<usize>,
    pub(super) rim_texture: Option<usize>,
    pub(super) outline_width_texture: Option<usize>,
    pub(super) outline_width_texture_uses_red: bool,
    pub(super) normal_scale: f32,
    pub(super) matcap_color: [f32; 3],
    pub(super) rim_lighting_mix: f32,
}

impl Default for CpuMtoonMaterial {
    fn default() -> Self {
        Self {
            enabled: false,
            shade_color: [0.0; 3],
            shading_shift: 0.0,
            shading_toony: 0.9,
            gi_equalization: 0.9,
            parametric_rim_color: [0.0; 3],
            rim_fresnel_power: 5.0,
            rim_lift: 0.0,
            emissive_color: [0.0; 3],
            outline_width: 0.0,
            outline_color: [0.0; 3],
            outline_lighting_mix: 1.0,
            transparent_with_z_write: false,
            render_queue_offset: 0,
            uv_scroll: [0.0; 2],
            uv_rotation: 0.0,
            shade_texture: None,
            normal_texture: None,
            matcap_texture: None,
            rim_texture: None,
            outline_width_texture: None,
            outline_width_texture_uses_red: false,
            normal_scale: 1.0,
            matcap_color: [1.0; 3],
            rim_lighting_mix: 1.0,
        }
    }
}

impl CpuMtoonMaterial {
    fn has_uv_animation(self) -> bool {
        self.enabled
            && (self.uv_scroll[0].abs() > f32::EPSILON
                || self.uv_scroll[1].abs() > f32::EPSILON
                || self.uv_rotation.abs() > f32::EPSILON)
    }
}

#[derive(Clone, Debug)]
pub(super) struct CpuDraw {
    pub(super) vertices: Range<u32>,
    pub(super) material_index: usize,
    pub(super) center_z: f32,
    pub(super) node_index: usize,
    pub(super) skin_index: Option<usize>,
    pub(super) morph_targets: Vec<CpuMorphTarget>,
}

#[derive(Debug)]
pub(super) struct CpuNode {
    pub(super) rest: NodeTransform,
    pub(super) parent: Option<usize>,
    pub(super) active: bool,
}

#[derive(Debug)]
pub(super) struct CpuSkin {
    pub(super) joints: Vec<usize>,
    pub(super) inverse_bind: Vec<Mat4>,
}

struct CpuSceneGraph {
    nodes: Vec<CpuNode>,
    traversal: Vec<usize>,
    worlds: Vec<Option<Mat4>>,
}

struct SceneBuildInput<'a> {
    gltf: &'a Gltf,
    blob: &'a [u8],
    root: &'a Value,
    worlds: &'a [Option<Mat4>],
    nodes: Vec<CpuNode>,
    traversal: Vec<usize>,
    skins: Vec<CpuSkin>,
    animations: Vec<CpuAnimationClip>,
    expressions: CpuExpressionSet,
    look_at: Option<CpuLookAt>,
    constraints: Vec<CpuNodeConstraint>,
    springs: Vec<CpuSpring>,
    colliders: Vec<CpuCollider>,
    scene_id: u64,
    version_label: &'static str,
}

#[derive(Debug)]
pub(in crate::components::avatar) struct CpuVrmScene {
    pub(super) scene_id: u64,
    pub(super) vertices: Vec<VrmVertex>,
    pub(super) draws: Vec<CpuDraw>,
    pub(super) materials: Vec<CpuMaterial>,
    pub(super) textures: Vec<CpuTexture>,
    pub(super) center: [f32; 3],
    pub(super) extent: [f32; 3],
    pub(in crate::components::avatar) version_label: &'static str,
    pub(super) front_direction: f32,
    pub(super) nodes: Vec<CpuNode>,
    pub(super) traversal: Vec<usize>,
    pub(super) skins: Vec<CpuSkin>,
    pub(super) animations: Vec<CpuAnimationClip>,
    animation_label: Option<String>,
    pub(super) expressions: CpuExpressionSet,
    pub(super) look_at: Option<CpuLookAt>,
    pub(super) constraints: Vec<CpuNodeConstraint>,
    pub(super) springs: Vec<CpuSpring>,
    pub(super) colliders: Vec<CpuCollider>,
    pub(super) custom_shader_source: Option<std::sync::Arc<str>>,
    pub(in crate::components::avatar) custom_shader_label: Option<String>,
    pub(in crate::components::avatar) custom_shader_error: Option<String>,
}

pub(super) struct CpuVrmFrame {
    pub(super) pose_matrices: Vec<Vec<[[f32; 4]; 4]>>,
    pub(super) morph_weights: HashMap<(usize, usize), f32>,
}

impl CpuVrmScene {
    pub(in crate::components::avatar) fn needs_continuous_update(&self) -> bool {
        !self.animations.is_empty()
            || !self.expressions.expressions.is_empty()
            || self.look_at.is_some()
            || !self.springs.is_empty()
            || self
                .materials
                .iter()
                .any(|material| material.mtoon.has_uv_animation())
    }

    pub(in crate::components::avatar) fn animation_label(&self) -> Option<&str> {
        self.animation_label.as_deref()
    }

    pub(super) fn pose_matrices(&self, time: f32) -> Vec<Vec<[[f32; 4]; 4]>> {
        let mut state = VrmRuntimeState::default();
        self.evaluate_frame(
            FrameInput {
                time,
                crossfade_seconds: 0.25,
                expression: "",
                look_yaw_degrees: 0.0,
                look_pitch_degrees: 0.0,
                spring_bone_enabled: false,
                look_at_enabled: false,
            },
            &mut state,
        )
        .pose_matrices
    }

    pub(super) fn evaluate_frame(
        &self,
        input: FrameInput<'_>,
        state: &mut VrmRuntimeState,
    ) -> CpuVrmFrame {
        let frame = super::runtime::evaluate_frame(
            RuntimeRig {
                nodes: &self.nodes,
                traversal: &self.traversal,
                animations: &self.animations,
                expressions: &self.expressions,
                look_at: self.look_at.as_ref(),
                constraints: &self.constraints,
                springs: &self.springs,
                colliders: &self.colliders,
            },
            input,
            state,
        );
        let worlds = frame.worlds;
        let pose_matrices = self
            .draws
            .iter()
            .map(|draw| {
                if let Some(skin_index) = draw.skin_index {
                    let skin = &self.skins[skin_index];
                    skin.joints
                        .iter()
                        .zip(&skin.inverse_bind)
                        .map(|(joint, inverse_bind)| {
                            (worlds[*joint] * *inverse_bind).to_cols_array_2d()
                        })
                        .collect()
                } else {
                    vec![worlds[draw.node_index].to_cols_array_2d()]
                }
            })
            .collect();
        CpuVrmFrame {
            pose_matrices,
            morph_weights: frame.morph_weights,
        }
    }
}

pub(super) fn decode(
    path: &str,
    animation_paths: &[String],
    shader_path: &str,
    scene_id: u64,
) -> Result<CpuVrmScene, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("failed to inspect VRM model {path}: {error}"))?;
    if metadata.len() > MAX_VRM_FILE_BYTES {
        return Err(format!(
            "VRM model exceeds the {} MiB file limit",
            MAX_VRM_FILE_BYTES / 1024 / 1024
        ));
    }

    let file = std::fs::File::open(path)
        .map_err(|error| format!("failed to open VRM model {path}: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len().min(MAX_VRM_FILE_BYTES) as usize);
    file.take(MAX_VRM_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read VRM model {path}: {error}"))?;
    if bytes.len() as u64 > MAX_VRM_FILE_BYTES {
        return Err(format!(
            "VRM model exceeds the {} MiB file limit",
            MAX_VRM_FILE_BYTES / 1024 / 1024
        ));
    }

    let mut scene = decode_bytes(&bytes, animation_paths, scene_id)?;
    if !shader_path.is_empty() {
        match super::custom_shader::load(shader_path) {
            Ok(shader) => {
                scene.custom_shader_source = Some(shader.source);
                scene.custom_shader_label = Some(shader.label);
            }
            Err(error) => scene.custom_shader_error = Some(error),
        }
    }
    Ok(scene)
}

fn decode_bytes(
    bytes: &[u8],
    animation_paths: &[String],
    scene_id: u64,
) -> Result<CpuVrmScene, String> {
    let root = parse_glb_root(bytes)?;
    let node_count = root
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if node_count == 0 || node_count > MAX_NODES {
        return Err(format!("VRM node count must be between 1 and {MAX_NODES}"));
    }
    validate_required_extensions(&root)?;
    let version_label = validate_vrm_extension(&root, node_count)?;

    let sanitized = glb_without_validated_required_extensions(bytes, &root)?;
    let gltf = Gltf::from_slice(&sanitized).map_err(|error| format!("invalid VRM GLB: {error}"))?;
    if gltf.nodes().len() != node_count {
        return Err("VRM node table changed during validation".to_string());
    }
    for source in gltf.buffers().map(|buffer| buffer.source()) {
        if matches!(source, buffer::Source::Uri(_)) {
            return Err("VRM external buffer URIs are not allowed".to_string());
        }
    }
    for image in gltf.images() {
        if matches!(image.source(), gltf_image::Source::Uri { .. }) {
            return Err("VRM external image URIs are not allowed".to_string());
        }
    }

    let blob = gltf
        .blob
        .as_deref()
        .ok_or_else(|| "VRM must contain an embedded GLB binary chunk".to_string())?;
    let mut buffers = gltf.buffers();
    let buffer = buffers
        .next()
        .ok_or_else(|| "VRM must declare one embedded GLB buffer".to_string())?;
    if buffers.next().is_some() || !matches!(buffer.source(), buffer::Source::Bin) {
        return Err("VRM must declare exactly one embedded GLB buffer".to_string());
    }
    if buffer.length() > blob.len() {
        return Err("VRM embedded buffer length exceeds the GLB binary chunk".to_string());
    }
    for view in gltf.views() {
        validate_buffer_view(&view, blob.len())?;
    }
    validate_mtoon_materials(&root, version_label)?;
    let CpuSceneGraph {
        nodes,
        traversal,
        worlds,
    } = collect_scene_graph(&gltf)?;
    let skins = collect_skins(&gltf, &worlds, blob)?;
    let humanoid = humanoid_bindings(&root, version_label, node_count)?;
    let expressions = decode_expressions(&root, &gltf, version_label)?;
    let look_at = decode_look_at(&root, &humanoid, version_label)?;
    let constraints = decode_node_constraints(&root, &nodes)?;
    let (springs, colliders) = decode_spring_bones(&root, &nodes, version_label)?;
    let animations = if animation_paths.is_empty() {
        let active = nodes.iter().map(|node| node.active).collect::<Vec<_>>();
        super::animation::decode_clips(&gltf, blob, Some(&active), None)?
    } else {
        let mut clips = Vec::new();
        for animation_path in animation_paths {
            clips.extend(decode_vrma(
                animation_path.trim(),
                &nodes,
                &worlds,
                &humanoid,
            )?);
            super::animation::validate_clip_collection(&clips)?;
        }
        clips
    };
    build_scene(SceneBuildInput {
        gltf: &gltf,
        blob,
        root: &root,
        worlds: &worlds,
        nodes,
        traversal,
        skins,
        animations,
        expressions,
        look_at,
        constraints,
        springs,
        colliders,
        scene_id,
        version_label,
    })
}

fn parse_glb_root(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err("VRM must be a binary glTF 2.0 file".to_string());
    }
    let version = read_u32(bytes, 4)?;
    if version != 2 {
        return Err(format!("VRM GLB version {version} is not supported"));
    }
    let declared_length = usize::try_from(read_u32(bytes, 8)?)
        .map_err(|_| "VRM GLB length is too large".to_string())?;
    if declared_length != bytes.len() {
        return Err("VRM GLB declared length does not match the file".to_string());
    }
    let json_length = usize::try_from(read_u32(bytes, 12)?)
        .map_err(|_| "VRM JSON chunk is too large".to_string())?;
    if read_u32(bytes, 16)? != 0x4E4F_534A {
        return Err("VRM GLB first chunk must be JSON".to_string());
    }
    let json_end = 20usize
        .checked_add(json_length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "VRM JSON chunk is truncated".to_string())?;

    serde_json::from_slice(&bytes[20..json_end])
        .map_err(|error| format!("invalid VRM JSON chunk: {error}"))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "VRM GLB header is truncated".to_string())?;
    let mut value = [0_u8; 4];
    value.copy_from_slice(&bytes[offset..end]);
    Ok(u32::from_le_bytes(value))
}

fn glb_without_validated_required_extensions(
    bytes: &[u8],
    root: &Value,
) -> Result<Vec<u8>, String> {
    let original_json_length = usize::try_from(read_u32(bytes, 12)?)
        .map_err(|_| "VRM JSON chunk is too large".to_string())?;
    let tail_start = 20usize
        .checked_add(original_json_length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "VRM JSON chunk is truncated".to_string())?;
    let mut sanitized_root = root.clone();
    if let Some(object) = sanitized_root.as_object_mut() {
        object.remove("extensionsRequired");
    }
    let mut json = serde_json::to_vec(&sanitized_root)
        .map_err(|error| format!("failed to validate VRM JSON: {error}"))?;
    while json.len() & 3 != 0 {
        json.push(b' ');
    }
    let total_length = 12usize
        .checked_add(8)
        .and_then(|length| length.checked_add(json.len()))
        .and_then(|length| length.checked_add(bytes.len() - tail_start))
        .ok_or_else(|| "sanitized VRM GLB length overflowed".to_string())?;
    let total_length =
        u32::try_from(total_length).map_err(|_| "sanitized VRM GLB is too large".to_string())?;
    let json_length =
        u32::try_from(json.len()).map_err(|_| "sanitized VRM JSON is too large".to_string())?;

    let mut sanitized = Vec::with_capacity(total_length as usize);
    sanitized.extend_from_slice(b"glTF");
    sanitized.extend_from_slice(&2_u32.to_le_bytes());
    sanitized.extend_from_slice(&total_length.to_le_bytes());
    sanitized.extend_from_slice(&json_length.to_le_bytes());
    sanitized.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
    sanitized.extend_from_slice(&json);
    sanitized.extend_from_slice(&bytes[tail_start..]);
    Ok(sanitized)
}

fn validate_required_extensions(root: &Value) -> Result<(), String> {
    validate_required_extension_names(root, &SUPPORTED_REQUIRED_EXTENSIONS, "VRM")
}

fn validate_vrm_extension(root: &Value, node_count: usize) -> Result<&'static str, String> {
    let extensions = root
        .get("extensions")
        .and_then(Value::as_object)
        .ok_or_else(|| "VRM extension object is missing".to_string())?;

    if let Some(vrm) = extensions.get("VRMC_vrm") {
        validate_vrm_one(vrm, root, node_count)?;
        return Ok("VRM 1.0");
    }
    if let Some(vrm) = extensions.get("VRM") {
        validate_vrm_zero(vrm, root, node_count)?;
        return Ok("VRM 0.x");
    }
    Err("GLB does not contain a VRMC_vrm or legacy VRM extension".to_string())
}

fn validate_vrm_one(vrm: &Value, root: &Value, node_count: usize) -> Result<(), String> {
    let vrm = vrm
        .as_object()
        .ok_or_else(|| "VRMC_vrm must be an object".to_string())?;
    if vrm.get("specVersion").and_then(Value::as_str) != Some("1.0") {
        return Err("only VRM 1.0 is supported for VRMC_vrm models".to_string());
    }
    let meta = vrm
        .get("meta")
        .and_then(Value::as_object)
        .ok_or_else(|| "VRM 1.0 meta object is required".to_string())?;
    required_non_empty_string(meta, "name", "VRM 1.0 meta")?;
    required_non_empty_string(meta, "licenseUrl", "VRM 1.0 meta")?;
    let authors = meta
        .get("authors")
        .and_then(Value::as_array)
        .filter(|authors| !authors.is_empty())
        .ok_or_else(|| "VRM 1.0 meta.authors must be a non-empty array".to_string())?;
    if authors.iter().any(|author| {
        author
            .as_str()
            .is_none_or(|author| author.trim().is_empty())
    }) {
        return Err("VRM 1.0 meta.authors must contain non-empty strings".to_string());
    }
    let bones = vrm
        .get("humanoid")
        .and_then(|humanoid| humanoid.get("humanBones"))
        .and_then(Value::as_object)
        .ok_or_else(|| "VRM 1.0 humanoid.humanBones object is required".to_string())?;
    let mut assigned = HashSet::new();
    let mut bindings = HashMap::new();
    for bone in REQUIRED_VRM_ONE_HUMANOID_BONES {
        let node = bones
            .get(bone)
            .and_then(|binding| binding.get("node"))
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("VRM 1.0 required humanoid bone {bone} is missing"))?;
        let node = validate_bone_node(bone, node, node_count, &mut assigned)?;
        bindings.insert(bone.to_string(), node);
    }
    validate_humanoid_hierarchy(root, &bindings, &REQUIRED_VRM_ONE_HIERARCHY)
}

fn validate_vrm_zero(vrm: &Value, root: &Value, node_count: usize) -> Result<(), String> {
    let vrm = vrm
        .as_object()
        .ok_or_else(|| "legacy VRM extension must be an object".to_string())?;
    let version = vrm
        .get("specVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| "legacy VRM specVersion is required".to_string())?;
    if version != "0.0" {
        return Err(format!("legacy VRM version {version} is not supported"));
    }
    let meta = vrm
        .get("meta")
        .and_then(Value::as_object)
        .ok_or_else(|| "legacy VRM meta object is required".to_string())?;
    for field in ["title", "author", "licenseName"] {
        if meta.get(field).is_some_and(|value| !value.is_string()) {
            return Err(format!("legacy VRM meta.{field} must be a string"));
        }
    }
    let bones = vrm
        .get("humanoid")
        .and_then(|humanoid| humanoid.get("humanBones"))
        .and_then(Value::as_array)
        .ok_or_else(|| "legacy VRM humanoid.humanBones array is required".to_string())?;
    let by_name: HashMap<&str, u64> = bones
        .iter()
        .filter_map(|bone| Some((bone.get("bone")?.as_str()?, bone.get("node")?.as_u64()?)))
        .collect();
    let mut assigned = HashSet::new();
    let mut bindings = HashMap::new();
    for bone in REQUIRED_VRM_ZERO_HUMANOID_BONES {
        let node = *by_name
            .get(bone)
            .ok_or_else(|| format!("legacy VRM required humanoid bone {bone} is missing"))?;
        let node = validate_bone_node(bone, node, node_count, &mut assigned)?;
        bindings.insert(bone.to_string(), node);
    }
    validate_humanoid_hierarchy(root, &bindings, &REQUIRED_VRM_ZERO_HIERARCHY)
}

fn humanoid_bindings(
    root: &Value,
    version_label: &str,
    node_count: usize,
) -> Result<HashMap<String, usize>, String> {
    let extensions = root
        .get("extensions")
        .and_then(Value::as_object)
        .ok_or_else(|| "VRM extension object is missing".to_string())?;
    if version_label == "VRM 1.0" {
        let bones = extensions
            .get("VRMC_vrm")
            .and_then(|vrm| vrm.get("humanoid"))
            .and_then(|humanoid| humanoid.get("humanBones"))
            .and_then(Value::as_object)
            .ok_or_else(|| "VRM 1.0 humanoid.humanBones object is required".to_string())?;
        bones
            .iter()
            .filter_map(|(bone, binding)| binding.get("node").map(|node| (bone, node)))
            .map(|(bone, node)| {
                Ok((
                    bone.clone(),
                    json_node_index(node, node_count, &format!("humanoid bone {bone}"))?,
                ))
            })
            .collect()
    } else {
        let bones = extensions
            .get("VRM")
            .and_then(|vrm| vrm.get("humanoid"))
            .and_then(|humanoid| humanoid.get("humanBones"))
            .and_then(Value::as_array)
            .ok_or_else(|| "legacy VRM humanoid.humanBones array is required".to_string())?;
        bones
            .iter()
            .filter_map(|binding| Some((binding.get("bone")?.as_str()?, binding.get("node")?)))
            .map(|(bone, node)| {
                Ok((
                    bone.to_string(),
                    json_node_index(node, node_count, &format!("humanoid bone {bone}"))?,
                ))
            })
            .collect()
    }
}

fn decode_expressions(
    root: &Value,
    gltf: &Gltf,
    version_label: &str,
) -> Result<CpuExpressionSet, String> {
    if version_label == "VRM 0.x" {
        return decode_legacy_expressions(root, gltf);
    }
    let Some(expressions) = root
        .get("extensions")
        .and_then(|extensions| extensions.get("VRMC_vrm"))
        .and_then(|vrm| vrm.get("expressions"))
        .and_then(Value::as_object)
    else {
        return Ok(CpuExpressionSet::default());
    };
    let mut result = CpuExpressionSet::default();
    for collection in ["preset", "custom"] {
        let Some(entries) = expressions.get(collection).and_then(Value::as_object) else {
            continue;
        };
        for (name, value) in entries {
            if result.expressions.contains_key(name) {
                return Err(format!("VRM expression {name} is defined more than once"));
            }
            result
                .expressions
                .insert(name.clone(), decode_expression(name, value, gltf)?);
        }
    }
    Ok(result)
}

fn decode_expression(name: &str, value: &Value, gltf: &Gltf) -> Result<CpuExpression, String> {
    let expression = value
        .as_object()
        .ok_or_else(|| format!("VRM expression {name} must be an object"))?;
    let is_binary = expression.get("isBinary").map_or(Ok(false), |value| {
        value
            .as_bool()
            .ok_or_else(|| format!("VRM expression {name}.isBinary must be a boolean"))
    })?;
    let mut morph_binds = Vec::new();
    if let Some(bindings) = expression.get("morphTargetBinds") {
        let bindings = bindings
            .as_array()
            .ok_or_else(|| format!("VRM expression {name}.morphTargetBinds must be an array"))?;
        for binding in bindings {
            let binding = binding.as_object().ok_or_else(|| {
                format!("VRM expression {name} morph target binding must be an object")
            })?;
            let node = json_index_field(binding, "node", gltf.nodes().len(), "expression node")?;
            let target = json_usize_field(binding, "index", "expression morph target")?;
            validate_morph_binding(gltf, node, target, name)?;
            let weight = object_f32_range(binding, "weight", 1.0, 0.0, 1.0, "expression")?;
            morph_binds.push(MorphBind {
                node,
                target,
                weight,
            });
        }
    }
    Ok(CpuExpression {
        is_binary,
        morph_binds,
        override_blink: decode_expression_override(expression, "overrideBlink", name)?,
        override_look_at: decode_expression_override(expression, "overrideLookAt", name)?,
        override_mouth: decode_expression_override(expression, "overrideMouth", name)?,
    })
}

fn decode_legacy_expressions(root: &Value, gltf: &Gltf) -> Result<CpuExpressionSet, String> {
    let Some(groups) = root
        .get("extensions")
        .and_then(|extensions| extensions.get("VRM"))
        .and_then(|vrm| vrm.get("blendShapeMaster"))
        .and_then(|master| master.get("blendShapeGroups"))
        .and_then(Value::as_array)
    else {
        return Ok(CpuExpressionSet::default());
    };
    let mesh_nodes = gltf
        .nodes()
        .filter_map(|node| Some((node.mesh()?.index(), node.index())));
    let mut by_mesh = HashMap::<usize, Vec<usize>>::new();
    for (mesh, node) in mesh_nodes {
        by_mesh.entry(mesh).or_default().push(node);
    }
    let mut result = CpuExpressionSet::default();
    for (group_index, group) in groups.iter().enumerate() {
        let group = group
            .as_object()
            .ok_or_else(|| format!("legacy blendShapeGroups[{group_index}] must be an object"))?;
        let raw_name = group
            .get("presetName")
            .and_then(Value::as_str)
            .filter(|name| *name != "unknown")
            .or_else(|| group.get("name").and_then(Value::as_str))
            .unwrap_or("custom");
        let name = canonical_legacy_expression_name(raw_name);
        let mut morph_binds = Vec::new();
        if let Some(bindings) = group.get("binds").and_then(Value::as_array) {
            for binding in bindings {
                let binding = binding
                    .as_object()
                    .ok_or_else(|| format!("legacy expression {name} binding must be an object"))?;
                let mesh = json_usize_field(binding, "mesh", "legacy expression mesh")?;
                let target = json_usize_field(binding, "index", "legacy expression morph target")?;
                let weight =
                    object_f32_range(binding, "weight", 100.0, 0.0, 100.0, "legacy expression")?
                        / 100.0;
                for node in by_mesh.get(&mesh).into_iter().flatten() {
                    validate_morph_binding(gltf, *node, target, &name)?;
                    morph_binds.push(MorphBind {
                        node: *node,
                        target,
                        weight,
                    });
                }
            }
        }
        result.expressions.insert(
            name,
            CpuExpression {
                is_binary: group
                    .get("isBinary")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                morph_binds,
                override_blink: ExpressionOverride::None,
                override_look_at: ExpressionOverride::None,
                override_mouth: ExpressionOverride::None,
            },
        );
    }
    Ok(result)
}

fn canonical_legacy_expression_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "a" => "aa",
        "i" => "ih",
        "u" => "ou",
        "e" => "ee",
        "o" => "oh",
        "blink_l" => "blinkLeft",
        "blink_r" => "blinkRight",
        "lookup" => "lookUp",
        "lookdown" => "lookDown",
        "lookleft" => "lookLeft",
        "lookright" => "lookRight",
        "joy" => "happy",
        "sorrow" => "sad",
        "fun" => "relaxed",
        "neutral" => "neutral",
        "blink" => "blink",
        "angry" => "angry",
        _ => name,
    }
    .to_string()
}

fn validate_morph_binding(
    gltf: &Gltf,
    node_index: usize,
    target_index: usize,
    expression: &str,
) -> Result<(), String> {
    let node = gltf.nodes().nth(node_index).ok_or_else(|| {
        format!("VRM expression {expression} references missing node {node_index}")
    })?;
    let mesh = node.mesh().ok_or_else(|| {
        format!("VRM expression {expression} node {node_index} does not contain a mesh")
    })?;
    if mesh
        .primitives()
        .any(|primitive| target_index >= primitive.morph_targets().count())
    {
        return Err(format!(
            "VRM expression {expression} references missing morph target {target_index}"
        ));
    }
    Ok(())
}

fn decode_expression_override(
    expression: &serde_json::Map<String, Value>,
    key: &str,
    name: &str,
) -> Result<ExpressionOverride, String> {
    match expression
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("none")
    {
        "none" => Ok(ExpressionOverride::None),
        "block" => Ok(ExpressionOverride::Block),
        "blend" => Ok(ExpressionOverride::Blend),
        value => Err(format!(
            "VRM expression {name}.{key} has invalid value {value}"
        )),
    }
}

fn decode_look_at(
    root: &Value,
    humanoid: &HashMap<String, usize>,
    version_label: &str,
) -> Result<Option<CpuLookAt>, String> {
    let (value, legacy) = if version_label == "VRM 1.0" {
        (
            root.get("extensions")
                .and_then(|extensions| extensions.get("VRMC_vrm"))
                .and_then(|vrm| vrm.get("lookAt")),
            false,
        )
    } else {
        (
            root.get("extensions")
                .and_then(|extensions| extensions.get("VRM"))
                .and_then(|vrm| vrm.get("firstPerson")),
            true,
        )
    };
    let Some(value) = value else { return Ok(None) };
    let object = value
        .as_object()
        .ok_or_else(|| "VRM lookAt must be an object".to_string())?;
    let kind = if legacy {
        match object
            .get("lookAtTypeName")
            .and_then(Value::as_str)
            .unwrap_or("Bone")
        {
            "Bone" => LookAtKind::Bone,
            "BlendShape" => LookAtKind::Expression,
            other => return Err(format!("legacy VRM lookAtTypeName {other} is invalid")),
        }
    } else {
        match object.get("type").and_then(Value::as_str).unwrap_or("bone") {
            "bone" => LookAtKind::Bone,
            "expression" => LookAtKind::Expression,
            other => return Err(format!("VRM lookAt type {other} is invalid")),
        }
    };
    let map = |modern: &str, legacy_name: &str| {
        let value = object.get(if legacy { legacy_name } else { modern });
        decode_range_map(value, legacy)
    };
    Ok(Some(CpuLookAt {
        kind,
        left_eye: humanoid.get("leftEye").copied(),
        right_eye: humanoid.get("rightEye").copied(),
        horizontal_inner: map("rangeMapHorizontalInner", "lookAtHorizontalInner")?,
        horizontal_outer: map("rangeMapHorizontalOuter", "lookAtHorizontalOuter")?,
        vertical_down: map("rangeMapVerticalDown", "lookAtVerticalDown")?,
        vertical_up: map("rangeMapVerticalUp", "lookAtVerticalUp")?,
    }))
}

fn decode_range_map(value: Option<&Value>, legacy: bool) -> Result<RangeMap, String> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Ok(RangeMap::default());
    };
    let input_key = if legacy { "xRange" } else { "inputMaxValue" };
    let output_key = if legacy { "yRange" } else { "outputScale" };
    Ok(RangeMap {
        input_max_degrees: object_f32_range(
            object,
            input_key,
            90.0,
            0.0,
            180.0,
            "lookAt range map",
        )?,
        output_scale: object_f32_range(object, output_key, 10.0, 0.0, 180.0, "lookAt range map")?,
    })
}

fn decode_node_constraints(
    root: &Value,
    nodes: &[CpuNode],
) -> Result<Vec<CpuNodeConstraint>, String> {
    let json_nodes = root
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "VRM node array is missing".to_string())?;
    let mut constraints = Vec::new();
    for (destination, node) in json_nodes.iter().enumerate() {
        let Some(extension) = node
            .get("extensions")
            .and_then(|extensions| extensions.get("VRMC_node_constraint"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        if extension.get("specVersion").and_then(Value::as_str) != Some("1.0") {
            return Err(format!(
                "VRM node {destination} supports only VRMC_node_constraint 1.0"
            ));
        }
        let constraint = extension
            .get("constraint")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("VRM node {destination} constraint object is missing"))?;
        let populated = ["roll", "aim", "rotation"]
            .into_iter()
            .filter(|kind| constraint.contains_key(*kind))
            .collect::<Vec<_>>();
        if populated.len() != 1 {
            return Err(format!(
                "VRM node {destination} constraint must contain exactly one kind"
            ));
        }
        let kind_name = populated[0];
        let value = constraint[kind_name]
            .as_object()
            .ok_or_else(|| format!("VRM node {destination} {kind_name} constraint is invalid"))?;
        let source = json_index_field(value, "source", nodes.len(), "constraint source")?;
        if source == destination || !nodes[source].active || !nodes[destination].active {
            return Err(format!(
                "VRM node {destination} constraint source/destination must be distinct active nodes"
            ));
        }
        let weight = object_f32_range(value, "weight", 1.0, 0.0, 1.0, "node constraint")?;
        let kind = match kind_name {
            "rotation" => ConstraintKind::Rotation,
            "roll" => ConstraintKind::Roll(parse_roll_axis(
                value.get("rollAxis").and_then(Value::as_str),
            )?),
            "aim" => ConstraintKind::Aim(parse_aim_axis(
                value.get("aimAxis").and_then(Value::as_str),
            )?),
            _ => unreachable!(),
        };
        constraints.push(CpuNodeConstraint {
            destination,
            source,
            weight,
            kind,
        });
    }
    order_constraints(constraints, nodes)
}

fn order_constraints(
    constraints: Vec<CpuNodeConstraint>,
    nodes: &[CpuNode],
) -> Result<Vec<CpuNodeConstraint>, String> {
    let by_destination = constraints
        .iter()
        .enumerate()
        .map(|(index, constraint)| (constraint.destination, index))
        .collect::<HashMap<_, _>>();
    let mut ordered = Vec::with_capacity(constraints.len());
    let mut marks = vec![0_u8; constraints.len()];
    fn visit(
        index: usize,
        constraints: &[CpuNodeConstraint],
        nodes: &[CpuNode],
        by_destination: &HashMap<usize, usize>,
        marks: &mut [u8],
        ordered: &mut Vec<CpuNodeConstraint>,
    ) -> Result<(), String> {
        match marks[index] {
            2 => return Ok(()),
            1 => return Err("VRM node constraints contain a circular dependency".to_string()),
            _ => {}
        }
        marks[index] = 1;
        let constraint = constraints[index];
        let mut dependencies = Vec::new();
        match constraint.kind {
            ConstraintKind::Rotation | ConstraintKind::Roll(_) => {
                if let Some(source_constraint) = by_destination.get(&constraint.source) {
                    dependencies.push(*source_constraint);
                }
            }
            ConstraintKind::Aim(_) => {
                for (dependency, candidate) in constraints.iter().enumerate() {
                    if dependency != index
                        && (is_strict_ancestor(nodes, candidate.destination, constraint.source)
                            || is_strict_ancestor(
                                nodes,
                                candidate.destination,
                                constraint.destination,
                            ))
                    {
                        dependencies.push(dependency);
                    }
                }
            }
        }
        dependencies.sort_unstable();
        dependencies.dedup();
        for dependency in dependencies {
            visit(
                dependency,
                constraints,
                nodes,
                by_destination,
                marks,
                ordered,
            )?;
        }
        marks[index] = 2;
        ordered.push(constraints[index]);
        Ok(())
    }
    for index in 0..constraints.len() {
        visit(
            index,
            &constraints,
            nodes,
            &by_destination,
            &mut marks,
            &mut ordered,
        )?;
    }
    Ok(ordered)
}

fn is_strict_ancestor(nodes: &[CpuNode], ancestor: usize, descendant: usize) -> bool {
    ancestor != descendant && is_descendant(nodes, ancestor, descendant)
}

fn parse_roll_axis(value: Option<&str>) -> Result<Vec3, String> {
    match value {
        Some("X") => Ok(Vec3::X),
        Some("Y") => Ok(Vec3::Y),
        Some("Z") => Ok(Vec3::Z),
        _ => Err("VRM roll constraint axis must be X, Y, or Z".to_string()),
    }
}

fn parse_aim_axis(value: Option<&str>) -> Result<Vec3, String> {
    match value {
        Some("PositiveX") => Ok(Vec3::X),
        Some("NegativeX") => Ok(Vec3::NEG_X),
        Some("PositiveY") => Ok(Vec3::Y),
        Some("NegativeY") => Ok(Vec3::NEG_Y),
        Some("PositiveZ") => Ok(Vec3::Z),
        Some("NegativeZ") => Ok(Vec3::NEG_Z),
        _ => Err("VRM aim constraint axis is invalid".to_string()),
    }
}

fn decode_spring_bones(
    root: &Value,
    nodes: &[CpuNode],
    version_label: &str,
) -> Result<(Vec<CpuSpring>, Vec<CpuCollider>), String> {
    if version_label == "VRM 0.x" {
        return decode_legacy_spring_bones(root, nodes);
    }
    let Some(extension) = root
        .get("extensions")
        .and_then(|extensions| extensions.get("VRMC_springBone"))
        .and_then(Value::as_object)
    else {
        return Ok((Vec::new(), Vec::new()));
    };
    if extension.get("specVersion").and_then(Value::as_str) != Some("1.0") {
        return Err("only VRMC_springBone 1.0 is supported".to_string());
    }
    let mut colliders = Vec::new();
    if let Some(values) = extension.get("colliders").and_then(Value::as_array) {
        for (index, value) in values.iter().enumerate() {
            let value = value
                .as_object()
                .ok_or_else(|| format!("SpringBone collider {index} must be an object"))?;
            let node = json_index_field(value, "node", nodes.len(), "SpringBone collider node")?;
            if !nodes[node].active {
                return Err(format!(
                    "SpringBone collider {index} references an inactive node"
                ));
            }
            let shape = value
                .get("shape")
                .and_then(Value::as_object)
                .ok_or_else(|| format!("SpringBone collider {index} shape is missing"))?;
            let sphere = shape.get("sphere").and_then(Value::as_object);
            let capsule = shape.get("capsule").and_then(Value::as_object);
            let shape = match (sphere, capsule) {
                (Some(sphere), None) => ColliderShape::Sphere {
                    offset: optional_vec3(sphere.get("offset"), Vec3::ZERO, "sphere offset")?,
                    radius: object_f32_range(
                        sphere,
                        "radius",
                        0.0,
                        0.0,
                        1000.0,
                        "sphere collider",
                    )?,
                },
                (None, Some(capsule)) => ColliderShape::Capsule {
                    offset: optional_vec3(capsule.get("offset"), Vec3::ZERO, "capsule offset")?,
                    tail: optional_vec3(capsule.get("tail"), Vec3::ZERO, "capsule tail")?,
                    radius: object_f32_range(
                        capsule,
                        "radius",
                        0.0,
                        0.0,
                        1000.0,
                        "capsule collider",
                    )?,
                },
                _ => {
                    return Err(format!(
                        "SpringBone collider {index} must contain exactly one sphere or capsule"
                    ));
                }
            };
            colliders.push(CpuCollider { node, shape });
        }
    }
    let collider_groups = extension
        .get("colliderGroups")
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .enumerate()
                .map(|(index, group)| {
                    let values = group
                        .get("colliders")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            format!("SpringBone collider group {index} colliders are missing")
                        })?;
                    values
                        .iter()
                        .map(|value| {
                            value
                                .as_u64()
                                .and_then(|value| usize::try_from(value).ok())
                                .filter(|value| *value < colliders.len())
                                .ok_or_else(|| {
                                    format!(
                                        "SpringBone collider group {index} references a missing collider"
                                    )
                                })
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    let mut springs = Vec::new();
    let mut claimed_joints = HashSet::new();
    if let Some(values) = extension.get("springs").and_then(Value::as_array) {
        for (spring_index, value) in values.iter().enumerate() {
            let value = value
                .as_object()
                .ok_or_else(|| format!("SpringBone spring {spring_index} must be an object"))?;
            let joints = value
                .get("joints")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("SpringBone spring {spring_index} joints are missing"))?
                .iter()
                .map(|joint| decode_spring_joint(joint, nodes.len()))
                .collect::<Result<Vec<_>, String>>()?;
            if joints.len() < 2 {
                return Err(format!(
                    "SpringBone spring {spring_index} needs at least two joints"
                ));
            }
            if joints.iter().any(|joint| !nodes[joint.node].active) {
                return Err(format!(
                    "SpringBone spring {spring_index} references an inactive node"
                ));
            }
            let mut chain_joints = HashSet::new();
            for joint in &joints {
                if !chain_joints.insert(joint.node) || !claimed_joints.insert(joint.node) {
                    return Err(format!(
                        "SpringBone spring {spring_index} duplicates a joint used by a spring chain"
                    ));
                }
            }
            for pair in joints.windows(2) {
                if !is_descendant(nodes, pair[0].node, pair[1].node) {
                    return Err(format!(
                        "SpringBone spring {spring_index} joints must form an ancestor chain"
                    ));
                }
            }
            let center = value
                .get("center")
                .map(|_| {
                    json_index_field(value, "center", nodes.len(), "SpringBone center").and_then(
                        |center| {
                            if nodes[center].active
                                && is_descendant(nodes, center, joints[0].node)
                            {
                                Ok(center)
                            } else {
                                Err(format!(
                                    "SpringBone spring {spring_index} center must be its first joint or an active ancestor"
                                ))
                            }
                        },
                    )
                })
                .transpose()?;
            let mut collider_indices = Vec::new();
            if let Some(groups) = value.get("colliderGroups").and_then(Value::as_array) {
                for group in groups {
                    let index = group
                        .as_u64()
                        .and_then(|index| usize::try_from(index).ok())
                        .filter(|index| *index < collider_groups.len())
                        .ok_or_else(|| {
                            format!("SpringBone spring {spring_index} collider group is invalid")
                        })?;
                    collider_indices.extend_from_slice(&collider_groups[index]);
                }
            }
            collider_indices.sort_unstable();
            collider_indices.dedup();
            springs.push(CpuSpring {
                joints,
                collider_indices,
                center,
            });
        }
    }
    Ok((springs, colliders))
}

fn decode_spring_joint(value: &Value, node_count: usize) -> Result<CpuSpringJoint, String> {
    let value = value
        .as_object()
        .ok_or_else(|| "SpringBone joint must be an object".to_string())?;
    let gravity_direction = optional_vec3(
        value.get("gravityDir"),
        Vec3::new(0.0, -1.0, 0.0),
        "SpringBone gravityDir",
    )?;
    Ok(CpuSpringJoint {
        node: json_index_field(value, "node", node_count, "SpringBone joint node")?,
        hit_radius: object_f32_range(value, "hitRadius", 0.0, 0.0, 1000.0, "SpringBone")?,
        stiffness: object_f32_range(value, "stiffness", 1.0, 0.0, 1000.0, "SpringBone")?,
        gravity_power: object_f32_range(value, "gravityPower", 0.0, 0.0, 1000.0, "SpringBone")?,
        gravity_direction: if gravity_direction.length_squared() <= f32::EPSILON {
            Vec3::new(0.0, -1.0, 0.0)
        } else {
            gravity_direction.normalize()
        },
        drag_force: object_f32_range(value, "dragForce", 0.5, 0.0, 1.0, "SpringBone")?,
    })
}

fn decode_legacy_spring_bones(
    root: &Value,
    nodes: &[CpuNode],
) -> Result<(Vec<CpuSpring>, Vec<CpuCollider>), String> {
    let Some(secondary) = root
        .get("extensions")
        .and_then(|extensions| extensions.get("VRM"))
        .and_then(|vrm| vrm.get("secondaryAnimation"))
        .and_then(Value::as_object)
    else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut colliders = Vec::new();
    let mut groups = Vec::new();
    if let Some(values) = secondary.get("colliderGroups").and_then(Value::as_array) {
        for group in values {
            let group = group
                .as_object()
                .ok_or_else(|| "legacy SpringBone collider group must be an object".to_string())?;
            let node = json_index_field(group, "node", nodes.len(), "legacy collider node")?;
            if !nodes[node].active {
                return Err("legacy SpringBone collider references an inactive node".to_string());
            }
            let mut indices = Vec::new();
            for collider in group
                .get("colliders")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let collider = collider
                    .as_object()
                    .ok_or_else(|| "legacy SpringBone collider must be an object".to_string())?;
                let index = colliders.len();
                colliders.push(CpuCollider {
                    node,
                    shape: ColliderShape::Sphere {
                        offset: optional_vec3(
                            collider.get("offset"),
                            Vec3::ZERO,
                            "legacy collider offset",
                        )?,
                        radius: object_f32_range(
                            collider,
                            "radius",
                            0.0,
                            0.0,
                            1000.0,
                            "legacy collider",
                        )?,
                    },
                });
                indices.push(index);
            }
            groups.push(indices);
        }
    }
    let mut springs = Vec::new();
    if let Some(bone_groups) = secondary.get("boneGroups").and_then(Value::as_array) {
        for bone_group in bone_groups {
            let group = bone_group
                .as_object()
                .ok_or_else(|| "legacy SpringBone bone group must be an object".to_string())?;
            let roots = group
                .get("bones")
                .and_then(Value::as_array)
                .ok_or_else(|| "legacy SpringBone bones are missing".to_string())?;
            let template = decode_legacy_joint_template(group)?;
            let center = match group.get("center").and_then(Value::as_i64) {
                None | Some(-1) => None,
                Some(index) => usize::try_from(index)
                    .ok()
                    .filter(|index| *index < nodes.len() && nodes[*index].active)
                    .map(Some)
                    .ok_or_else(|| "legacy SpringBone center is invalid".to_string())?,
            };
            let mut collider_indices = Vec::new();
            for value in group
                .get("colliderGroups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let group_index = value
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|index| *index < groups.len())
                    .ok_or_else(|| {
                        "legacy SpringBone references an invalid collider group".to_string()
                    })?;
                collider_indices.extend(groups[group_index].iter().copied());
            }
            for root in roots {
                let root = root
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|value| *value < nodes.len())
                    .ok_or_else(|| "legacy SpringBone root is invalid".to_string())?;
                if !nodes[root].active
                    || center.is_some_and(|center| !is_descendant(nodes, center, root))
                {
                    return Err(
                        "legacy SpringBone root must be active and descend from its center"
                            .to_string(),
                    );
                }
                let mut chain = vec![root];
                while let Some(child) = nodes.iter().enumerate().find_map(|(index, node)| {
                    (node.parent == chain.last().copied()).then_some(index)
                }) {
                    if chain.contains(&child) {
                        break;
                    }
                    chain.push(child);
                }
                if chain.len() >= 2 {
                    springs.push(CpuSpring {
                        joints: chain
                            .into_iter()
                            .map(|node| CpuSpringJoint { node, ..template })
                            .collect(),
                        collider_indices: collider_indices.clone(),
                        center,
                    });
                }
            }
        }
    }
    Ok((springs, colliders))
}

fn decode_legacy_joint_template(
    value: &serde_json::Map<String, Value>,
) -> Result<CpuSpringJoint, String> {
    let gravity_direction = optional_vec3(
        value.get("gravityDir"),
        Vec3::new(0.0, -1.0, 0.0),
        "legacy SpringBone gravityDir",
    )?;
    Ok(CpuSpringJoint {
        node: 0,
        hit_radius: object_f32_range(value, "hitRadius", 0.0, 0.0, 1000.0, "legacy SpringBone")?,
        stiffness: object_f32_range(value, "stiffiness", 1.0, 0.0, 1000.0, "legacy SpringBone")?,
        gravity_power: object_f32_range(
            value,
            "gravityPower",
            0.0,
            0.0,
            1000.0,
            "legacy SpringBone",
        )?,
        gravity_direction: gravity_direction.normalize_or_zero(),
        drag_force: object_f32_range(value, "dragForce", 0.5, 0.0, 1.0, "legacy SpringBone")?,
    })
}

fn is_descendant(nodes: &[CpuNode], ancestor: usize, descendant: usize) -> bool {
    let mut current = Some(descendant);
    for _ in 0..nodes.len() {
        let Some(node) = current else { return false };
        if node == ancestor {
            return true;
        }
        current = nodes[node].parent;
    }
    false
}

fn json_index_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
    count: usize,
    context: &str,
) -> Result<usize, String> {
    let index = json_usize_field(object, key, context)?;
    if index >= count {
        Err(format!("{context} references missing index {index}"))
    } else {
        Ok(index)
    }
}

fn json_usize_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<usize, String> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| format!("{context}.{key} must be an unsigned integer"))
}

fn object_f32_range(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: f32,
    minimum: f32,
    maximum: f32,
    context: &str,
) -> Result<f32, String> {
    let value = object.get(key).map_or(Ok(default), |value| {
        value
            .as_f64()
            .map(|value| value as f32)
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("{context}.{key} must be a finite number"))
    })?;
    if (minimum..=maximum).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "{context}.{key} must be between {minimum} and {maximum}"
        ))
    }
}

fn optional_vec3(value: Option<&Value>, default: Vec3, context: &str) -> Result<Vec3, String> {
    let Some(value) = value else {
        return Ok(default);
    };
    let values = value
        .as_array()
        .filter(|values| values.len() == 3)
        .ok_or_else(|| format!("{context} must be a three-number array"))?;
    let mut result = [0.0_f32; 3];
    for (index, output) in result.iter_mut().enumerate() {
        *output = values[index]
            .as_f64()
            .map(|value| value as f32)
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("{context} must contain finite numbers"))?;
    }
    Ok(Vec3::from_array(result))
}

fn validate_mtoon_materials(root: &Value, version_label: &str) -> Result<(), String> {
    let material_count = root
        .get("materials")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    for index in 0..material_count {
        let _ = decode_mtoon_material(root, Some(index), [0.0; 3], version_label)?;
    }
    Ok(())
}

fn decode_mtoon_material(
    root: &Value,
    material_index: Option<usize>,
    emissive_factor: [f32; 3],
    version_label: &str,
) -> Result<CpuMtoonMaterial, String> {
    let Some(material_index) = material_index else {
        return Ok(CpuMtoonMaterial::default());
    };
    if version_label == "VRM 1.0" {
        let Some(extension) = root
            .get("materials")
            .and_then(Value::as_array)
            .and_then(|materials| materials.get(material_index))
            .and_then(|material| material.get("extensions"))
            .and_then(|extensions| extensions.get("VRMC_materials_mtoon"))
        else {
            return Ok(CpuMtoonMaterial::default());
        };
        let extension = extension.as_object().ok_or_else(|| {
            format!("VRM material {material_index} VRMC_materials_mtoon must be an object")
        })?;
        if extension.get("specVersion").and_then(Value::as_str) != Some("1.0") {
            return Err(format!(
                "VRM material {material_index} supports only VRMC_materials_mtoon 1.0"
            ));
        }
        let outline_mode = json_string(extension, "outlineWidthMode", "none")?;
        if !matches!(
            outline_mode,
            "none" | "worldCoordinates" | "screenCoordinates"
        ) {
            return Err(format!(
                "VRM material {material_index} has invalid MToon outlineWidthMode"
            ));
        }
        let transparent_with_z_write = json_bool(extension, "transparentWithZWrite", false)?;
        let render_queue_offset = json_i32(extension, "renderQueueOffsetNumber", 0)?;
        let allowed_queue = if transparent_with_z_write {
            0..=9
        } else {
            -9..=0
        };
        let alpha_mode = root
            .get("materials")
            .and_then(Value::as_array)
            .and_then(|materials| materials.get(material_index))
            .and_then(|material| material.get("alphaMode"))
            .and_then(Value::as_str)
            .unwrap_or("OPAQUE");
        if alpha_mode == "BLEND" && !allowed_queue.contains(&render_queue_offset) {
            return Err(format!(
                "VRM material {material_index} MToon renderQueueOffsetNumber is out of range"
            ));
        }
        if alpha_mode != "BLEND" && render_queue_offset != 0 {
            return Err(format!(
                "VRM material {material_index} may offset the MToon render queue only in BLEND mode"
            ));
        }

        Ok(CpuMtoonMaterial {
            enabled: true,
            shade_color: json_vec3(extension, "shadeColorFactor", [0.0; 3])?,
            shading_shift: json_f32_range(extension, "shadingShiftFactor", 0.0, -1.0, 1.0)?,
            shading_toony: json_f32_range(extension, "shadingToonyFactor", 0.9, 0.0, 1.0)?,
            gi_equalization: json_f32_range(extension, "giEqualizationFactor", 0.9, 0.0, 1.0)?,
            parametric_rim_color: json_vec3(extension, "parametricRimColorFactor", [0.0; 3])?,
            rim_fresnel_power: json_f32_range(
                extension,
                "parametricRimFresnelPowerFactor",
                5.0,
                0.0,
                100.0,
            )?,
            rim_lift: json_f32_range(extension, "parametricRimLiftFactor", 0.0, -1.0, 1.0)?,
            emissive_color: finite_vec3(emissive_factor, "emissiveFactor")?,
            outline_width: if outline_mode == "none" {
                0.0
            } else {
                json_f32_range(extension, "outlineWidthFactor", 0.0, 0.0, 1.0)?
            },
            outline_color: json_vec3(extension, "outlineColorFactor", [0.0; 3])?,
            outline_lighting_mix: json_f32_range(
                extension,
                "outlineLightingMixFactor",
                1.0,
                0.0,
                1.0,
            )?,
            transparent_with_z_write,
            render_queue_offset,
            uv_scroll: [
                json_f32(extension, "uvAnimationScrollXSpeedFactor", 0.0)?,
                json_f32(extension, "uvAnimationScrollYSpeedFactor", 0.0)?,
            ],
            uv_rotation: json_f32(extension, "uvAnimationRotationSpeedFactor", 0.0)?,
            shade_texture: json_texture_index(root, extension, "shadeMultiplyTexture")?,
            normal_texture: None,
            matcap_texture: json_texture_index(root, extension, "matcapTexture")?,
            rim_texture: json_texture_index(root, extension, "rimMultiplyTexture")?,
            outline_width_texture: json_texture_index(
                root,
                extension,
                "outlineWidthMultiplyTexture",
            )?,
            outline_width_texture_uses_red: false,
            normal_scale: 1.0,
            matcap_color: json_vec3(extension, "matcapFactor", [1.0; 3])?,
            rim_lighting_mix: json_f32_range(extension, "rimLightingMixFactor", 1.0, 0.0, 1.0)?,
        })
    } else {
        decode_legacy_mtoon(root, material_index, emissive_factor)
    }
}

fn decode_legacy_mtoon(
    root: &Value,
    material_index: usize,
    emissive_factor: [f32; 3],
) -> Result<CpuMtoonMaterial, String> {
    let Some(property) = root
        .get("extensions")
        .and_then(|extensions| extensions.get("VRM"))
        .and_then(|vrm| vrm.get("materialProperties"))
        .and_then(Value::as_array)
        .and_then(|properties| properties.get(material_index))
    else {
        return Ok(CpuMtoonMaterial::default());
    };
    let property = property.as_object().ok_or_else(|| {
        format!("legacy VRM material property {material_index} must be an object")
    })?;
    if !property
        .get("shader")
        .and_then(Value::as_str)
        .is_some_and(|shader| shader.contains("MToon"))
    {
        return Ok(CpuMtoonMaterial::default());
    }
    let floats = property.get("floatProperties").and_then(Value::as_object);
    let vectors = property.get("vectorProperties").and_then(Value::as_object);
    let texture_properties = property.get("textureProperties").and_then(Value::as_object);
    let outline_mode = legacy_float(floats, "_OutlineWidthMode", 0.0)?;
    let render_queue = property
        .get("renderQueue")
        .and_then(Value::as_i64)
        .unwrap_or(3000);
    let render_queue_offset = i32::try_from((render_queue - 3000).clamp(-9, 9))
        .map_err(|_| "legacy MToon render queue is invalid".to_string())?;
    Ok(CpuMtoonMaterial {
        enabled: true,
        shade_color: legacy_vec3(vectors, "_ShadeColor", [0.0; 3])?,
        shading_shift: legacy_float(floats, "_ShadeShift", 0.0)?.clamp(-1.0, 1.0),
        shading_toony: legacy_float(floats, "_ShadeToony", 0.9)?.clamp(0.0, 1.0),
        gi_equalization: legacy_float(floats, "_IndirectLightIntensity", 0.9)?.clamp(0.0, 1.0),
        parametric_rim_color: legacy_vec3(vectors, "_RimColor", [0.0; 3])?,
        rim_fresnel_power: legacy_float(floats, "_RimFresnelPower", 5.0)?.max(0.0),
        rim_lift: legacy_float(floats, "_RimLift", 0.0)?.clamp(-1.0, 1.0),
        emissive_color: legacy_vec3(vectors, "_EmissionColor", emissive_factor)?,
        outline_width: if outline_mode <= 0.0 {
            0.0
        } else {
            legacy_float(floats, "_OutlineWidth", 0.0)?.clamp(0.0, 1.0)
        },
        outline_color: legacy_vec3(vectors, "_OutlineColor", [0.0; 3])?,
        outline_lighting_mix: legacy_float(floats, "_OutlineLightingMix", 1.0)?.clamp(0.0, 1.0),
        transparent_with_z_write: legacy_float(floats, "_ZWrite", 0.0)? > 0.5,
        render_queue_offset,
        uv_scroll: [
            legacy_float(floats, "_UvAnimScrollX", 0.0)?,
            legacy_float(floats, "_UvAnimScrollY", 0.0)?,
        ],
        uv_rotation: legacy_float(floats, "_UvAnimRotation", 0.0)?,
        shade_texture: legacy_texture_index(root, texture_properties, "_ShadeTexture")?,
        normal_texture: legacy_texture_index(root, texture_properties, "_BumpMap")?,
        matcap_texture: legacy_texture_index(root, texture_properties, "_SphereAdd")?,
        rim_texture: legacy_texture_index(root, texture_properties, "_RimTexture")?,
        outline_width_texture: legacy_texture_index(
            root,
            texture_properties,
            "_OutlineWidthTexture",
        )?,
        outline_width_texture_uses_red: true,
        normal_scale: legacy_float(floats, "_BumpScale", 1.0)?.max(0.0),
        matcap_color: legacy_vec3(vectors, "_SphereAdd", [1.0; 3])?,
        rim_lighting_mix: legacy_float(floats, "_RimLightingMix", 1.0)?.clamp(0.0, 1.0),
    })
}

fn json_texture_index(
    root: &Value,
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<usize>, String> {
    let Some(texture) = object.get(key) else {
        return Ok(None);
    };
    let texture = texture
        .as_object()
        .ok_or_else(|| format!("MToon {key} must be a texture info object"))?;
    let texture_count = root
        .get("textures")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    Ok(Some(json_index_field(
        texture,
        "index",
        texture_count,
        &format!("MToon {key}"),
    )?))
}

fn legacy_texture_index(
    root: &Value,
    object: Option<&serde_json::Map<String, Value>>,
    key: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = object.and_then(|object| object.get(key)) else {
        return Ok(None);
    };
    let texture_count = root
        .get("textures")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let index = value
        .as_i64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|index| *index < texture_count)
        .ok_or_else(|| format!("legacy MToon {key} texture index is invalid"))?;
    Ok(Some(index))
}

fn json_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
    default: &'a str,
) -> Result<&'a str, String> {
    object.get(key).map_or(Ok(default), |value| {
        value
            .as_str()
            .ok_or_else(|| format!("MToon {key} must be a string"))
    })
}

fn json_bool(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    object.get(key).map_or(Ok(default), |value| {
        value
            .as_bool()
            .ok_or_else(|| format!("MToon {key} must be a boolean"))
    })
}

fn json_i32(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: i32,
) -> Result<i32, String> {
    object.get(key).map_or(Ok(default), |value| {
        value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| format!("MToon {key} must be a 32-bit integer"))
    })
}

fn json_f32(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: f32,
) -> Result<f32, String> {
    object.get(key).map_or(Ok(default), |value| {
        let value = value
            .as_f64()
            .map(|value| value as f32)
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("MToon {key} must be a finite number"))?;
        Ok(value)
    })
}

fn json_f32_range(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: f32,
    min: f32,
    max: f32,
) -> Result<f32, String> {
    let value = json_f32(object, key, default)?;
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(format!("MToon {key} must be between {min} and {max}"))
    }
}

fn json_vec3(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: [f32; 3],
) -> Result<[f32; 3], String> {
    let Some(value) = object.get(key) else {
        return Ok(default);
    };
    value
        .as_array()
        .filter(|values| values.len() == 3)
        .ok_or_else(|| format!("MToon {key} must be a three-number array"))?
        .iter()
        .map(|value| {
            value
                .as_f64()
                .map(|value| value as f32)
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("MToon {key} must contain finite numbers"))
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| format!("MToon {key} must have three values"))
}

fn finite_vec3(value: [f32; 3], label: &str) -> Result<[f32; 3], String> {
    if value.iter().all(|value| value.is_finite()) {
        Ok(value)
    } else {
        Err(format!("MToon {label} contains NaN or infinity"))
    }
}

fn legacy_float(
    object: Option<&serde_json::Map<String, Value>>,
    key: &str,
    default: f32,
) -> Result<f32, String> {
    object
        .and_then(|object| object.get(key))
        .map_or(Ok(default), |value| {
            value
                .as_f64()
                .map(|value| value as f32)
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("legacy MToon {key} must be a finite number"))
        })
}

fn legacy_vec3(
    object: Option<&serde_json::Map<String, Value>>,
    key: &str,
    default: [f32; 3],
) -> Result<[f32; 3], String> {
    let Some(values) = object.and_then(|object| object.get(key)) else {
        return Ok(default);
    };
    let values = values
        .as_array()
        .filter(|values| values.len() >= 3)
        .ok_or_else(|| format!("legacy MToon {key} must contain at least three numbers"))?;
    let mut result = [0.0_f32; 3];
    for (index, result) in result.iter_mut().enumerate() {
        *result = values[index]
            .as_f64()
            .map(|value| value as f32)
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("legacy MToon {key} must contain finite numbers"))?;
    }
    Ok(result)
}

fn decode_vrma(
    path: &str,
    target_nodes: &[CpuNode],
    target_worlds: &[Option<Mat4>],
    target_humanoid: &HashMap<String, usize>,
) -> Result<Vec<CpuAnimationClip>, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("failed to inspect VRMA animation {path}: {error}"))?;
    if metadata.len() > MAX_VRMA_FILE_BYTES {
        return Err(format!(
            "VRMA animation exceeds the {} MiB file limit",
            MAX_VRMA_FILE_BYTES / 1024 / 1024
        ));
    }
    let file = std::fs::File::open(path)
        .map_err(|error| format!("failed to open VRMA animation {path}: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len().min(MAX_VRMA_FILE_BYTES) as usize);
    file.take(MAX_VRMA_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read VRMA animation {path}: {error}"))?;
    if bytes.len() as u64 > MAX_VRMA_FILE_BYTES {
        return Err(format!(
            "VRMA animation exceeds the {} MiB file limit",
            MAX_VRMA_FILE_BYTES / 1024 / 1024
        ));
    }

    let root = parse_glb_root(&bytes)?;
    let node_count = root
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if node_count == 0 || node_count > MAX_NODES {
        return Err(format!("VRMA node count must be between 1 and {MAX_NODES}"));
    }
    validate_required_extension_names(&root, &SUPPORTED_VRMA_REQUIRED_EXTENSIONS, "VRMA")?;
    let source_humanoid = validate_vrma_extension(&root, node_count)?;
    let sanitized = glb_without_validated_required_extensions(&bytes, &root)?;
    let gltf = Gltf::from_slice(&sanitized)
        .map_err(|error| format!("invalid VRMA binary glTF: {error}"))?;
    for source in gltf.buffers().map(|buffer| buffer.source()) {
        if matches!(source, buffer::Source::Uri(_)) {
            return Err("VRMA external buffer URIs are not allowed".to_string());
        }
    }
    let blob = gltf
        .blob
        .as_deref()
        .ok_or_else(|| "VRMA must contain an embedded GLB binary chunk".to_string())?;
    let mut buffers = gltf.buffers();
    let buffer = buffers
        .next()
        .ok_or_else(|| "VRMA must declare one embedded GLB buffer".to_string())?;
    if buffers.next().is_some() || !matches!(buffer.source(), buffer::Source::Bin) {
        return Err("VRMA must declare exactly one embedded GLB buffer".to_string());
    }
    if buffer.length() > blob.len() {
        return Err("VRMA embedded buffer length exceeds the GLB binary chunk".to_string());
    }
    for view in gltf.views() {
        validate_buffer_view(&view, blob.len())?;
    }
    let CpuSceneGraph {
        nodes: source_nodes,
        worlds: source_worlds,
        ..
    } = collect_scene_graph(&gltf)?;
    for (bone, node) in &source_humanoid {
        if !source_nodes.get(*node).is_some_and(|node| node.active) {
            return Err(format!(
                "VRMA humanoid bone {bone} is outside the active scene"
            ));
        }
    }

    let translation_scale = humanoid_translation_scale(
        &source_humanoid,
        &source_worlds,
        target_humanoid,
        target_worlds,
    );
    let mut retarget = HashMap::new();
    for (bone, source_node) in source_humanoid {
        let Some(target_node) = target_humanoid.get(&bone).copied() else {
            continue;
        };
        let source_rest = source_nodes
            .get(source_node)
            .ok_or_else(|| format!("VRMA bone {bone} references a missing source node"))?
            .rest;
        let target_rest = target_nodes
            .get(target_node)
            .ok_or_else(|| format!("VRM bone {bone} references a missing target node"))?
            .rest;
        retarget.insert(
            source_node,
            RetargetTarget {
                target_node,
                source_rest,
                target_rest,
                translation_scale,
                hips: bone == "hips",
            },
        );
    }

    super::animation::decode_clips(&gltf, blob, None, Some(&retarget))
}

fn validate_vrma_extension(
    root: &Value,
    node_count: usize,
) -> Result<HashMap<String, usize>, String> {
    let animation = root
        .get("extensions")
        .and_then(|extensions| extensions.get("VRMC_vrm_animation"))
        .and_then(Value::as_object)
        .ok_or_else(|| "VRMA VRMC_vrm_animation extension object is missing".to_string())?;
    if animation.get("specVersion").and_then(Value::as_str) != Some("1.0") {
        return Err("only VRMC_vrm_animation 1.0 is supported".to_string());
    }
    let bones = animation
        .get("humanoid")
        .and_then(|humanoid| humanoid.get("humanBones"))
        .and_then(Value::as_object)
        .ok_or_else(|| "VRMA humanoid.humanBones object is required".to_string())?;
    let mut bindings = HashMap::new();
    let mut assigned = HashSet::new();
    for (bone, binding) in bones {
        let node = binding
            .get("node")
            .ok_or_else(|| format!("VRMA humanoid bone {bone} node is missing"))?;
        let node = json_node_index(node, node_count, &format!("humanoid bone {bone}"))?;
        if !assigned.insert(node) {
            return Err(format!("VRMA humanoid bone {bone} reuses node {node}"));
        }
        bindings.insert(bone.clone(), node);
    }
    for bone in REQUIRED_VRM_ONE_HUMANOID_BONES {
        if !bindings.contains_key(bone) {
            return Err(format!("VRMA required humanoid bone {bone} is missing"));
        }
    }
    validate_humanoid_hierarchy(root, &bindings, &REQUIRED_VRM_ONE_HIERARCHY)?;
    Ok(bindings)
}

fn humanoid_translation_scale(
    source: &HashMap<String, usize>,
    source_worlds: &[Option<Mat4>],
    target: &HashMap<String, usize>,
    target_worlds: &[Option<Mat4>],
) -> f32 {
    let height = |bindings: &HashMap<String, usize>, worlds: &[Option<Mat4>]| {
        let hips = bindings
            .get("hips")
            .and_then(|node| worlds.get(*node))
            .and_then(|world| *world)?
            .transform_point3(Vec3::ZERO);
        let head = bindings
            .get("head")
            .and_then(|node| worlds.get(*node))
            .and_then(|world| *world)?
            .transform_point3(Vec3::ZERO);
        Some(hips.distance(head))
    };
    match (height(source, source_worlds), height(target, target_worlds)) {
        (Some(source), Some(target)) if source > f32::EPSILON && target.is_finite() => {
            (target / source).clamp(0.25, 4.0)
        }
        _ => 1.0,
    }
}

fn validate_required_extension_names(
    root: &Value,
    supported: &[&str],
    label: &str,
) -> Result<(), String> {
    let Some(required) = root.get("extensionsRequired") else {
        return Ok(());
    };
    let required = required
        .as_array()
        .ok_or_else(|| format!("{label} extensionsRequired must be an array"))?;
    for extension in required {
        let extension = extension
            .as_str()
            .ok_or_else(|| format!("{label} required extension name must be a string"))?;
        if !supported.contains(&extension) {
            return Err(format!(
                "{label} requires unsupported glTF extension {extension}"
            ));
        }
    }
    Ok(())
}

fn required_non_empty_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<(), String> {
    if object
        .get(field)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        Ok(())
    } else {
        Err(format!("{context}.{field} must be a non-empty string"))
    }
}

pub(super) fn validate_accessor_iteration(
    accessor: &gltf::Accessor<'_>,
    label: &str,
) -> Result<(), String> {
    if accessor.count() == 0 {
        return Err(format!("VRM {label} accessor must not be empty"));
    }
    if let Some(sparse) = accessor.sparse() {
        if sparse.count() == 0 || sparse.count() > accessor.count() {
            return Err(format!(
                "VRM {label} sparse accessor count must be between 1 and its accessor count"
            ));
        }
        let indices = sparse.indices();
        validate_view_data_range(
            &indices.view(),
            indices.offset(),
            indices
                .view()
                .stride()
                .unwrap_or_else(|| indices.index_type().size()),
            sparse.count(),
            indices.index_type().size(),
            &format!("{label} sparse indices"),
        )?;
        let values = sparse.values();
        validate_view_data_range(
            &values.view(),
            values.offset(),
            values.view().stride().unwrap_or_else(|| accessor.size()),
            sparse.count(),
            accessor.size(),
            &format!("{label} sparse values"),
        )?;
    }
    if let Some(view) = accessor.view() {
        validate_view_data_range(
            &view,
            accessor.offset(),
            view.stride().unwrap_or_else(|| accessor.size()),
            accessor.count(),
            accessor.size(),
            label,
        )?;
    } else if accessor.sparse().is_none() {
        return Err(format!(
            "VRM {label} accessor has neither base data nor sparse data"
        ));
    }
    Ok(())
}

fn validate_buffer_view(view: &gltf::buffer::View<'_>, blob_len: usize) -> Result<(), String> {
    let end = view
        .offset()
        .checked_add(view.length())
        .filter(|end| *end <= blob_len && *end <= view.buffer().length())
        .ok_or_else(|| format!("VRM buffer view {} is outside embedded data", view.index()))?;
    let _ = end;
    Ok(())
}

fn validate_view_data_range(
    view: &gltf::buffer::View<'_>,
    offset: usize,
    stride: usize,
    count: usize,
    element_size: usize,
    label: &str,
) -> Result<(), String> {
    if stride < element_size {
        return Err(format!(
            "VRM {label} accessor stride {stride} is smaller than element size {element_size}"
        ));
    }
    let end = stride
        .checked_mul(count - 1)
        .and_then(|length| offset.checked_add(length))
        .and_then(|last| last.checked_add(element_size))
        .filter(|end| *end <= view.length())
        .ok_or_else(|| format!("VRM {label} accessor is outside its buffer view"))?;
    let _ = end;
    Ok(())
}

fn validate_bone_node(
    bone: &str,
    node: u64,
    node_count: usize,
    assigned: &mut HashSet<usize>,
) -> Result<usize, String> {
    let node = usize::try_from(node).map_err(|_| format!("VRM bone {bone} node is too large"))?;
    if node >= node_count {
        return Err(format!("VRM bone {bone} references missing node {node}"));
    }
    if !assigned.insert(node) {
        return Err(format!("VRM bone {bone} reuses humanoid node {node}"));
    }
    Ok(node)
}

fn validate_humanoid_hierarchy(
    root: &Value,
    bindings: &HashMap<String, usize>,
    required_hierarchy: &[(&str, &str)],
) -> Result<(), String> {
    let nodes = root
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "VRM node array is missing".to_string())?;
    let scenes = root
        .get("scenes")
        .and_then(Value::as_array)
        .ok_or_else(|| "VRM scene array is missing".to_string())?;
    let scene_index = root.get("scene").and_then(Value::as_u64).unwrap_or(0);
    let scene_index = usize::try_from(scene_index)
        .map_err(|_| "VRM default scene index is too large".to_string())?;
    let scene_roots = scenes
        .get(scene_index)
        .and_then(|scene| scene.get("nodes"))
        .and_then(Value::as_array)
        .ok_or_else(|| "VRM active scene root nodes are missing".to_string())?;
    if scene_roots.is_empty() {
        return Err("VRM active scene must contain at least one root node".to_string());
    }

    let mut active = vec![false; nodes.len()];
    let mut parents = vec![None; nodes.len()];
    let mut stack = Vec::with_capacity(nodes.len());
    for root_node in scene_roots {
        let root_node = json_node_index(root_node, nodes.len(), "scene root")?;
        stack.push((root_node, 0_usize));
    }

    while let Some((node, depth)) = stack.pop() {
        if depth > MAX_NODE_DEPTH {
            return Err(format!("VRM node hierarchy exceeds depth {MAX_NODE_DEPTH}"));
        }
        if active[node] {
            return Err(format!(
                "VRM active scene node {node} is referenced more than once"
            ));
        }
        active[node] = true;

        let Some(children) = nodes[node].get("children") else {
            continue;
        };
        let children = children
            .as_array()
            .ok_or_else(|| format!("VRM node {node} children must be an array"))?;
        for child in children {
            let child = json_node_index(child, nodes.len(), "child")?;
            if parents[child].replace(node).is_some() {
                return Err(format!("VRM node {child} has more than one parent"));
            }
            stack.push((child, depth + 1));
        }
    }

    for (bone, node) in bindings {
        if !active[*node] {
            return Err(format!(
                "VRM humanoid bone {bone} is outside the active scene"
            ));
        }
    }
    let bones_by_node = bindings
        .iter()
        .map(|(bone, node)| (*node, bone.as_str()))
        .collect::<HashMap<_, _>>();
    for (ancestor_bone, descendant_bone) in required_hierarchy {
        let descendant = *bindings
            .get(*descendant_bone)
            .ok_or_else(|| format!("VRM humanoid bone {descendant_bone} is missing"))?;
        let mut current = parents[descendant];
        let mut nearest_humanoid_parent = None;
        for _ in 0..parents.len() {
            let Some(node) = current else {
                break;
            };
            if let Some(bone) = bones_by_node.get(&node) {
                nearest_humanoid_parent = Some(*bone);
                break;
            }
            current = parents[node];
        }
        if nearest_humanoid_parent != Some(*ancestor_bone) {
            return Err(format!(
                "VRM humanoid bone {descendant_bone} must have {ancestor_bone} as its nearest required humanoid parent"
            ));
        }
    }
    Ok(())
}

fn json_node_index(value: &Value, node_count: usize, context: &str) -> Result<usize, String> {
    let node = value
        .as_u64()
        .ok_or_else(|| format!("VRM {context} node index must be an unsigned integer"))?;
    let node =
        usize::try_from(node).map_err(|_| format!("VRM {context} node index is too large"))?;
    if node >= node_count {
        return Err(format!("VRM {context} references missing node {node}"));
    }
    Ok(node)
}

fn collect_scene_graph(gltf: &Gltf) -> Result<CpuSceneGraph, String> {
    let mut nodes = gltf
        .nodes()
        .map(|node| {
            Ok(CpuNode {
                rest: NodeTransform::from_node(node)?,
                parent: None,
                active: false,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    for node in gltf.nodes() {
        for child in node.children() {
            if nodes[child.index()].parent.replace(node.index()).is_some() {
                return Err(format!(
                    "VRM node {} has more than one parent",
                    child.index()
                ));
            }
        }
    }

    let mut worlds = vec![None; nodes.len()];
    let mut traversal = Vec::with_capacity(nodes.len());
    let scene = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .ok_or_else(|| "VRM scene is missing".to_string())?;
    for node in scene.nodes() {
        walk_node(
            node,
            Mat4::IDENTITY,
            0,
            &mut nodes,
            &mut traversal,
            &mut worlds,
        )?;
    }
    Ok(CpuSceneGraph {
        nodes,
        traversal,
        worlds,
    })
}

fn walk_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    depth: usize,
    nodes: &mut [CpuNode],
    traversal: &mut Vec<usize>,
    worlds: &mut [Option<Mat4>],
) -> Result<(), String> {
    if depth > MAX_NODE_DEPTH {
        return Err(format!("VRM node hierarchy exceeds depth {MAX_NODE_DEPTH}"));
    }
    if worlds[node.index()].is_some() {
        return Err(format!(
            "VRM node {} is referenced more than once in the active scene",
            node.index()
        ));
    }
    let local = nodes[node.index()].rest.matrix();
    let world = parent * local;
    if !world.to_cols_array().iter().all(|value| value.is_finite()) {
        return Err(format!(
            "VRM node {} has a non-finite transform",
            node.index()
        ));
    }
    nodes[node.index()].active = true;
    traversal.push(node.index());
    worlds[node.index()] = Some(world);
    for child in node.children() {
        walk_node(child, world, depth + 1, nodes, traversal, worlds)?;
    }
    Ok(())
}

fn build_scene(input: SceneBuildInput<'_>) -> Result<CpuVrmScene, String> {
    let SceneBuildInput {
        gltf,
        blob,
        root,
        worlds,
        nodes,
        traversal,
        skins,
        animations,
        expressions,
        look_at,
        constraints,
        springs,
        colliders,
        scene_id,
        version_label,
    } = input;
    let front_direction = if version_label == "VRM 0.x" {
        -1.0
    } else {
        1.0
    };
    let mut vertices = Vec::new();
    let mut draws = Vec::new();
    let mut materials = Vec::new();
    let mut textures = Vec::new();
    let mut texture_cache = HashMap::new();
    let mut texture_decode_bytes = 0_u64;
    let mut primitive_count = 0_usize;
    let mut triangle_count = 0_usize;
    let mut source_vertex_count = 0_usize;
    let mut morph_delta_count = 0_usize;
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for node in gltf.nodes() {
        let Some(mesh) = node.mesh() else {
            continue;
        };
        let Some(mesh_world) = worlds[node.index()] else {
            continue;
        };
        let skin_index = node.skin().map(|skin| skin.index());
        let skin_matrices = skin_index
            .map(|skin_index| {
                let skin = skins
                    .get(skin_index)
                    .ok_or_else(|| format!("VRM node references missing skin {skin_index}"))?;
                skin.joints
                    .iter()
                    .zip(&skin.inverse_bind)
                    .map(|(joint, inverse_bind)| {
                        worlds
                            .get(*joint)
                            .and_then(|world| *world)
                            .map(|world| world * *inverse_bind)
                            .ok_or_else(|| {
                                format!("VRM skin joint node {joint} is outside the active scene")
                            })
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?;

        for primitive in mesh.primitives() {
            primitive_count += 1;
            if primitive_count > MAX_PRIMITIVES {
                return Err(format!(
                    "VRM has more than {MAX_PRIMITIVES} mesh primitives"
                ));
            }
            if primitive.mode() != Mode::Triangles {
                continue;
            }

            let position_accessor = primitive
                .get(&Semantic::Positions)
                .ok_or_else(|| "VRM triangle primitive is missing POSITION data".to_string())?;
            validate_accessor_iteration(&position_accessor, "POSITION")?;
            if position_accessor.data_type() != DataType::F32
                || position_accessor.dimensions() != Dimensions::Vec3
                || position_accessor.normalized()
            {
                return Err("VRM POSITION accessor must be non-normalized F32 VEC3".to_string());
            }
            let position_count = position_accessor.count();
            source_vertex_count = source_vertex_count
                .checked_add(position_count)
                .ok_or_else(|| "VRM source vertex count overflowed".to_string())?;
            if source_vertex_count > MAX_SOURCE_VERTICES {
                return Err(format!(
                    "VRM has more than {MAX_SOURCE_VERTICES} source vertices"
                ));
            }
            if let Some(accessor) = primitive.indices() {
                validate_accessor_iteration(&accessor, "index")?;
                if accessor.dimensions() != Dimensions::Scalar
                    || !matches!(
                        accessor.data_type(),
                        DataType::U8 | DataType::U16 | DataType::U32
                    )
                    || accessor.normalized()
                {
                    return Err(
                        "VRM index accessor must be non-normalized U8/U16/U32 SCALAR".to_string(),
                    );
                }
                if accessor.count() > MAX_OUTPUT_VERTICES {
                    return Err(format!(
                        "VRM primitive index count exceeds {MAX_OUTPUT_VERTICES}"
                    ));
                }
            }

            let reader = primitive.reader(|buffer| match buffer.source() {
                buffer::Source::Bin => Some(blob),
                buffer::Source::Uri(_) => None,
            });
            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .ok_or_else(|| "VRM triangle primitive is missing POSITION data".to_string())?
                .collect();
            if positions.is_empty() {
                continue;
            }
            if positions
                .iter()
                .flatten()
                .any(|coordinate| !coordinate.is_finite())
            {
                return Err("VRM vertex position contains NaN or infinity".to_string());
            }
            let normals = if let Some(accessor) = primitive.get(&Semantic::Normals) {
                validate_accessor_iteration(&accessor, "NORMAL")?;
                if accessor.count() != positions.len()
                    || accessor.data_type() != DataType::F32
                    || accessor.dimensions() != Dimensions::Vec3
                    || accessor.normalized()
                {
                    return Err(
                        "VRM NORMAL accessor must be matching non-normalized F32 VEC3".to_string(),
                    );
                }
                let normals = reader
                    .read_normals()
                    .ok_or_else(|| "VRM NORMAL data could not be decoded".to_string())?
                    .map(Vec3::from_array)
                    .collect::<Vec<_>>();
                if normals
                    .iter()
                    .any(|normal| !normal.is_finite() || normal.length_squared() <= f32::EPSILON)
                {
                    return Err("VRM vertex normal is invalid".to_string());
                }
                Some(normals)
            } else {
                None
            };
            let tangents = if let Some(accessor) = primitive.get(&Semantic::Tangents) {
                validate_accessor_iteration(&accessor, "TANGENT")?;
                if accessor.count() != positions.len()
                    || accessor.data_type() != DataType::F32
                    || accessor.dimensions() != Dimensions::Vec4
                    || accessor.normalized()
                {
                    return Err(
                        "VRM TANGENT accessor must be matching non-normalized F32 VEC4".to_string(),
                    );
                }
                let values = reader
                    .read_tangents()
                    .ok_or_else(|| "VRM TANGENT data could not be decoded".to_string())?
                    .collect::<Vec<_>>();
                if values.iter().flatten().any(|value| !value.is_finite()) {
                    return Err("VRM vertex tangent contains NaN or infinity".to_string());
                }
                Some(values)
            } else {
                None
            };

            let indices: Vec<u32> = if primitive.indices().is_some() {
                reader
                    .read_indices()
                    .ok_or_else(|| "VRM index data could not be decoded".to_string())?
                    .into_u32()
                    .collect()
            } else {
                (0..positions.len() as u32).collect()
            };
            if !indices.chunks_exact(3).remainder().is_empty() {
                return Err("VRM triangle index count must be divisible by three".to_string());
            }
            triangle_count = triangle_count
                .checked_add(indices.len() / 3)
                .ok_or_else(|| "VRM triangle count overflowed".to_string())?;
            if triangle_count > MAX_TRIANGLES {
                return Err(format!("VRM has more than {MAX_TRIANGLES} triangles"));
            }

            let material = primitive.material();
            let pbr = material.pbr_metallic_roughness();
            let base_texture = pbr.base_color_texture();
            let tex_coord_set = base_texture
                .as_ref()
                .map_or(0, |texture| texture.tex_coord());
            let tex_coords = match primitive.get(&Semantic::TexCoords(tex_coord_set)) {
                Some(accessor) => {
                    validate_accessor_iteration(&accessor, "TEXCOORD")?;
                    if accessor.count() != positions.len() {
                        return Err(
                            "VRM texture coordinate count does not match positions".to_string()
                        );
                    }
                    let valid_type = match accessor.data_type() {
                        DataType::F32 => !accessor.normalized(),
                        DataType::U8 | DataType::U16 => accessor.normalized(),
                        _ => false,
                    };
                    if accessor.dimensions() != Dimensions::Vec2 || !valid_type {
                        return Err(
                            "VRM TEXCOORD accessor must be F32 or normalized U8/U16 VEC2"
                                .to_string(),
                        );
                    }
                    Some(
                        reader
                            .read_tex_coords(tex_coord_set)
                            .ok_or_else(|| {
                                "VRM texture coordinates could not be decoded".to_string()
                            })?
                            .into_f32()
                            .collect::<Vec<_>>(),
                    )
                }
                None => None,
            };

            let (joints, weights) = if skin_matrices.is_some() {
                let joints_accessor = primitive
                    .get(&Semantic::Joints(0))
                    .ok_or_else(|| "VRM skinned primitive is missing JOINTS_0".to_string())?;
                let weights_accessor = primitive
                    .get(&Semantic::Weights(0))
                    .ok_or_else(|| "VRM skinned primitive is missing WEIGHTS_0".to_string())?;
                validate_accessor_iteration(&joints_accessor, "JOINTS_0")?;
                validate_accessor_iteration(&weights_accessor, "WEIGHTS_0")?;
                if joints_accessor.count() != positions.len()
                    || weights_accessor.count() != positions.len()
                {
                    return Err(
                        "VRM skinned primitive needs matching JOINTS_0 and WEIGHTS_0 data"
                            .to_string(),
                    );
                }
                if joints_accessor.dimensions() != Dimensions::Vec4
                    || !matches!(joints_accessor.data_type(), DataType::U8 | DataType::U16)
                    || joints_accessor.normalized()
                {
                    return Err(
                        "VRM JOINTS_0 accessor must be non-normalized U8/U16 VEC4".to_string()
                    );
                }
                let valid_weights = match weights_accessor.data_type() {
                    DataType::F32 => !weights_accessor.normalized(),
                    DataType::U8 | DataType::U16 => weights_accessor.normalized(),
                    _ => false,
                };
                if weights_accessor.dimensions() != Dimensions::Vec4 || !valid_weights {
                    return Err(
                        "VRM WEIGHTS_0 accessor must be F32 or normalized U8/U16 VEC4".to_string(),
                    );
                }
                let joints = reader
                    .read_joints(0)
                    .ok_or_else(|| "VRM JOINTS_0 data could not be decoded".to_string())?
                    .into_u16()
                    .collect::<Vec<_>>();
                let weights = reader
                    .read_weights(0)
                    .ok_or_else(|| "VRM WEIGHTS_0 data could not be decoded".to_string())?
                    .into_f32()
                    .collect::<Vec<_>>();
                (Some(joints), Some(weights))
            } else {
                (None, None)
            };
            let relevant_morph_targets = expressions
                .expressions
                .values()
                .flat_map(|expression| &expression.morph_binds)
                .filter(|binding| binding.node == node.index())
                .map(|binding| binding.target)
                .collect::<HashSet<_>>();
            let mut source_morph_targets = Vec::new();
            for (target_index, (position_values, normal_values, tangent_values)) in
                reader.read_morph_targets().enumerate()
            {
                if !relevant_morph_targets.contains(&target_index) {
                    continue;
                }
                let target = primitive
                    .morph_targets()
                    .nth(target_index)
                    .ok_or_else(|| "VRM morph target table changed during decoding".to_string())?;
                for (accessor, label) in [
                    (target.positions(), "morph POSITION"),
                    (target.normals(), "morph NORMAL"),
                    (target.tangents(), "morph TANGENT"),
                ] {
                    if let Some(accessor) = accessor {
                        validate_accessor_iteration(&accessor, label)?;
                        if accessor.count() != positions.len()
                            || accessor.data_type() != DataType::F32
                            || accessor.dimensions() != Dimensions::Vec3
                            || accessor.normalized()
                        {
                            return Err(format!(
                                "VRM {label} accessor must be matching non-normalized F32 VEC3"
                            ));
                        }
                    }
                }
                let positions_delta = position_values
                    .map(|values| values.map(Vec3::from_array).collect::<Vec<_>>())
                    .unwrap_or_else(|| vec![Vec3::ZERO; positions.len()]);
                let normals_delta = normal_values
                    .map(|values| values.map(Vec3::from_array).collect::<Vec<_>>())
                    .unwrap_or_else(|| vec![Vec3::ZERO; positions.len()]);
                let tangents_delta = tangent_values
                    .map(|values| values.map(Vec3::from_array).collect::<Vec<_>>())
                    .unwrap_or_else(|| vec![Vec3::ZERO; positions.len()]);
                if positions_delta
                    .iter()
                    .chain(&normals_delta)
                    .chain(&tangents_delta)
                    .any(|value| !value.is_finite())
                {
                    return Err("VRM morph target contains NaN or infinity".to_string());
                }
                source_morph_targets.push((
                    target_index,
                    positions_delta,
                    normals_delta,
                    tangents_delta,
                ));
            }

            let texture_index = if let Some(texture) = base_texture {
                Some(load_gltf_texture(
                    gltf,
                    texture.texture().index(),
                    blob,
                    &mut texture_decode_bytes,
                    &mut texture_cache,
                    &mut textures,
                )?)
            } else {
                None
            };
            let alpha_mode = match material.alpha_mode() {
                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                gltf::material::AlphaMode::Mask => AlphaMode::Mask,
                gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            };
            let material_index = materials.len();
            let mut mtoon = decode_mtoon_material(
                root,
                material.index(),
                material.emissive_factor(),
                version_label,
            )?;
            if let Some(normal) = material.normal_texture() {
                mtoon.normal_texture = Some(normal.texture().index());
                mtoon.normal_scale = normal.scale();
            }
            for texture in [
                &mut mtoon.shade_texture,
                &mut mtoon.normal_texture,
                &mut mtoon.matcap_texture,
                &mut mtoon.rim_texture,
                &mut mtoon.outline_width_texture,
            ] {
                if let Some(source_index) = *texture {
                    *texture = Some(load_gltf_texture(
                        gltf,
                        source_index,
                        blob,
                        &mut texture_decode_bytes,
                        &mut texture_cache,
                        &mut textures,
                    )?);
                }
            }
            materials.push(CpuMaterial {
                base_color: pbr.base_color_factor(),
                texture_index,
                alpha_mode,
                alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
                mtoon,
            });

            let start_index = vertices.len();
            let start = u32::try_from(start_index)
                .map_err(|_| "VRM vertex buffer is too large".to_string())?;
            let mut triangle_depths = Vec::with_capacity(indices.len() / 3);
            let mut expanded_source_indices = Vec::with_capacity(indices.len());
            let mut draw_min_z = f32::INFINITY;
            let mut draw_max_z = f32::NEG_INFINITY;
            for triangle in indices.chunks_exact(3) {
                let mut local_points = [Vec3::ZERO; 3];
                let mut rest_points = [Vec3::ZERO; 3];
                let mut uvs = [[0.0_f32; 2]; 3];
                let mut vertex_joints = [[0_u16; 4]; 3];
                let mut vertex_weights = [[1.0_f32, 0.0, 0.0, 0.0]; 3];
                let mut source_indices = [0_usize; 3];
                for corner in 0..3 {
                    let index = usize::try_from(triangle[corner])
                        .map_err(|_| "VRM vertex index is too large".to_string())?;
                    source_indices[corner] = index;
                    let position = positions
                        .get(index)
                        .ok_or_else(|| format!("VRM index {index} is outside POSITION data"))?;
                    local_points[corner] = Vec3::from_array(*position);
                    rest_points[corner] = transform_position(
                        local_points[corner],
                        index,
                        mesh_world,
                        skin_matrices.as_deref(),
                        joints.as_deref(),
                        weights.as_deref(),
                    )?;
                    if let Some(coords) = &tex_coords {
                        uvs[corner] = coords[index];
                    }
                    if let (Some(joints), Some(weights)) = (&joints, &weights) {
                        let (normalized_joints, normalized_weights) = normalized_influences(
                            *joints
                                .get(index)
                                .ok_or_else(|| "VRM joint data is missing".to_string())?,
                            *weights
                                .get(index)
                                .ok_or_else(|| "VRM weight data is missing".to_string())?,
                            skin_matrices.as_deref().unwrap_or_default().len(),
                        )?;
                        vertex_joints[corner] = normalized_joints;
                        vertex_weights[corner] = normalized_weights;
                    }
                }

                let rest_face =
                    (rest_points[1] - rest_points[0]).cross(rest_points[2] - rest_points[0]);
                let local_face =
                    (local_points[1] - local_points[0]).cross(local_points[2] - local_points[0]);
                if !rest_face.is_finite()
                    || rest_face.length_squared() <= f32::EPSILON
                    || !local_face.is_finite()
                    || local_face.length_squared() <= f32::EPSILON
                {
                    continue;
                }
                let fallback_normal = local_face.normalize();
                let fallback_tangent = triangle_tangent(local_points, uvs, fallback_normal);
                for corner in 0..3 {
                    min = min.min(rest_points[corner]);
                    max = max.max(rest_points[corner]);
                    draw_min_z = draw_min_z.min(rest_points[corner].z);
                    draw_max_z = draw_max_z.max(rest_points[corner].z);
                    let normal = normals
                        .as_ref()
                        .and_then(|normals| normals.get(source_indices[corner]))
                        .copied()
                        .unwrap_or(fallback_normal)
                        .normalize();
                    let tangent = tangents
                        .as_ref()
                        .and_then(|tangents| tangents.get(source_indices[corner]))
                        .copied()
                        .unwrap_or(fallback_tangent);
                    vertices.push(VrmVertex {
                        position: local_points[corner].to_array(),
                        normal: normal.to_array(),
                        uv: uvs[corner],
                        tangent,
                        joints: vertex_joints[corner],
                        weights: vertex_weights[corner],
                    });
                    expanded_source_indices.push(source_indices[corner]);
                }
                triangle_depths.push(
                    rest_points.iter().map(|point| point.z).sum::<f32>() * (front_direction / 3.0),
                );
                if vertices.len() > MAX_OUTPUT_VERTICES {
                    return Err(format!(
                        "VRM expanded vertex count exceeds {MAX_OUTPUT_VERTICES}"
                    ));
                }
            }
            let end = u32::try_from(vertices.len())
                .map_err(|_| "VRM vertex buffer is too large".to_string())?;
            if end > start {
                let mut morph_targets = source_morph_targets
                    .into_iter()
                    .map(
                        |(target_index, positions, normals, tangents)| CpuMorphTarget {
                            target_index,
                            deltas: expanded_source_indices
                                .iter()
                                .map(|source| CpuMorphDelta {
                                    position: positions[*source].extend(0.0).to_array(),
                                    normal: normals[*source].extend(0.0).to_array(),
                                    tangent: tangents[*source].extend(0.0).to_array(),
                                })
                                .collect(),
                        },
                    )
                    .collect::<Vec<_>>();
                let draw_morph_delta_count = morph_targets
                    .iter()
                    .map(|target| target.deltas.len())
                    .sum::<usize>();
                morph_delta_count = morph_delta_count
                    .checked_add(draw_morph_delta_count)
                    .ok_or_else(|| "VRM expression morph data size overflowed".to_string())?;
                if morph_delta_count > MAX_MORPH_DELTAS {
                    return Err(format!(
                        "VRM expression morph data exceeds {MAX_MORPH_DELTAS} expanded deltas"
                    ));
                }
                if alpha_mode == AlphaMode::Blend {
                    sort_blended_triangles(
                        &mut vertices[start_index..],
                        &triangle_depths,
                        &mut morph_targets,
                    );
                }
                draws.push(CpuDraw {
                    vertices: start..end,
                    material_index,
                    center_z: (draw_min_z + draw_max_z) * 0.5,
                    node_index: node.index(),
                    skin_index,
                    morph_targets,
                });
            }
        }
    }

    if vertices.is_empty() || draws.is_empty() {
        return Err("VRM active scene contains no renderable triangles".to_string());
    }
    let extent = max - min;
    if !extent.is_finite() || extent.x.max(extent.y).max(extent.z) <= f32::EPSILON {
        return Err("VRM model bounds are empty or invalid".to_string());
    }

    Ok(CpuVrmScene {
        scene_id,
        vertices,
        draws,
        materials,
        textures,
        center: ((min + max) * 0.5).to_array(),
        extent: extent.to_array(),
        version_label,
        front_direction,
        nodes,
        traversal,
        skins,
        animation_label: if animations.is_empty() {
            None
        } else if animations.len() == 1 {
            Some(animations[0].source_label.to_string())
        } else {
            Some(format!("{} blended clips", animations.len()))
        },
        animations,
        expressions,
        look_at,
        constraints,
        springs,
        colliders,
        custom_shader_source: None,
        custom_shader_label: None,
        custom_shader_error: None,
    })
}

fn triangle_tangent(points: [Vec3; 3], uvs: [[f32; 2]; 3], normal: Vec3) -> [f32; 4] {
    let edge_a = points[1] - points[0];
    let edge_b = points[2] - points[0];
    let uv_a = Vec3::new(uvs[1][0] - uvs[0][0], uvs[1][1] - uvs[0][1], 0.0);
    let uv_b = Vec3::new(uvs[2][0] - uvs[0][0], uvs[2][1] - uvs[0][1], 0.0);
    let determinant = uv_a.x * uv_b.y - uv_a.y * uv_b.x;
    let tangent = if determinant.abs() > f32::EPSILON {
        ((edge_a * uv_b.y - edge_b * uv_a.y) / determinant).normalize_or_zero()
    } else {
        let reference = if normal.y.abs() < 0.99 {
            Vec3::Y
        } else {
            Vec3::X
        };
        reference.cross(normal).normalize_or_zero()
    };
    let bitangent = if determinant.abs() > f32::EPSILON {
        ((edge_b * uv_a.x - edge_a * uv_b.x) / determinant).normalize_or_zero()
    } else {
        normal.cross(tangent)
    };
    [
        tangent.x,
        tangent.y,
        tangent.z,
        if normal.cross(tangent).dot(bitangent) < 0.0 {
            -1.0
        } else {
            1.0
        },
    ]
}

fn sort_blended_triangles(
    vertices: &mut [VrmVertex],
    depths: &[f32],
    morph_targets: &mut [CpuMorphTarget],
) {
    let mut order = (0..depths.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| depths[*left].total_cmp(&depths[*right]));
    let original_vertices = vertices.to_vec();
    for (target_index, source_index) in order.iter().copied().enumerate() {
        vertices[target_index * 3..target_index * 3 + 3]
            .copy_from_slice(&original_vertices[source_index * 3..source_index * 3 + 3]);
    }
    for morph_target in morph_targets {
        let original = morph_target.deltas.clone();
        for (target_index, source_index) in order.iter().copied().enumerate() {
            morph_target.deltas[target_index * 3..target_index * 3 + 3]
                .copy_from_slice(&original[source_index * 3..source_index * 3 + 3]);
        }
    }
}

fn collect_skins(
    gltf: &Gltf,
    worlds: &[Option<Mat4>],
    blob: &[u8],
) -> Result<Vec<CpuSkin>, String> {
    gltf.skins()
        .map(|skin| {
            let joints = skin.joints().map(|joint| joint.index()).collect::<Vec<_>>();
            if joints.is_empty() || joints.len() > MAX_SKIN_JOINTS {
                return Err(format!(
                    "VRM skin joint count must be between 1 and {MAX_SKIN_JOINTS}"
                ));
            }
            if let Some(accessor) = skin.inverse_bind_matrices() {
                validate_accessor_iteration(&accessor, "inverse bind matrix")?;
                if accessor.count() != joints.len() {
                    return Err(
                        "VRM inverse bind matrix count does not match skin joints".to_string()
                    );
                }
                if accessor.data_type() != DataType::F32
                    || accessor.dimensions() != Dimensions::Mat4
                    || accessor.normalized()
                {
                    return Err(
                        "VRM inverse bind accessor must be non-normalized F32 MAT4".to_string()
                    );
                }
            }
            let reader = skin.reader(|buffer| match buffer.source() {
                buffer::Source::Bin => Some(blob),
                buffer::Source::Uri(_) => None,
            });
            let inverse_bind: Vec<Mat4> = if skin.inverse_bind_matrices().is_some() {
                reader
                    .read_inverse_bind_matrices()
                    .ok_or_else(|| "VRM inverse bind matrix data could not be decoded".to_string())?
                    .map(|matrix| Mat4::from_cols_array_2d(&matrix))
                    .collect()
            } else {
                vec![Mat4::IDENTITY; joints.len()]
            };
            if inverse_bind.len() != joints.len() {
                return Err("VRM inverse bind matrix count does not match skin joints".to_string());
            }
            for (joint, inverse_bind) in joints.iter().zip(&inverse_bind) {
                let world = worlds
                    .get(*joint)
                    .and_then(|matrix| *matrix)
                    .ok_or_else(|| {
                        format!("VRM skin joint node {joint} is outside the active scene")
                    })?;
                let matrix = world * *inverse_bind;
                if !matrix.to_cols_array().iter().all(|value| value.is_finite()) {
                    return Err("VRM skin matrix contains NaN or infinity".to_string());
                }
            }
            Ok(CpuSkin {
                joints,
                inverse_bind,
            })
        })
        .collect()
}

fn transform_position(
    position: Vec3,
    vertex_index: usize,
    mesh_world: Mat4,
    skin_matrices: Option<&[Mat4]>,
    joints: Option<&[[u16; 4]]>,
    weights: Option<&[[f32; 4]]>,
) -> Result<Vec3, String> {
    let Some(skin_matrices) = skin_matrices else {
        let transformed = mesh_world.transform_point3(position);
        return finite_position(transformed);
    };
    let joints = joints
        .and_then(|values| values.get(vertex_index))
        .ok_or_else(|| "VRM joint data is missing".to_string())?;
    let weights = weights
        .and_then(|values| values.get(vertex_index))
        .ok_or_else(|| "VRM weight data is missing".to_string())?;
    let mut transformed = Vec3::ZERO;
    let mut weight_sum = 0.0_f32;
    for influence in 0..4 {
        let weight = weights[influence];
        if !weight.is_finite() || weight < 0.0 {
            return Err("VRM skin weight is invalid".to_string());
        }
        if weight <= f32::EPSILON {
            continue;
        }
        let matrix = skin_matrices
            .get(usize::from(joints[influence]))
            .ok_or_else(|| "VRM vertex references a missing skin joint".to_string())?;
        transformed += matrix.transform_point3(position) * weight;
        weight_sum += weight;
    }
    if weight_sum <= f32::EPSILON {
        transformed = mesh_world.transform_point3(position);
    } else {
        transformed /= weight_sum;
    }
    finite_position(transformed)
}

fn normalized_influences(
    joints: [u16; 4],
    mut weights: [f32; 4],
    joint_count: usize,
) -> Result<([u16; 4], [f32; 4]), String> {
    let mut weight_sum = 0.0_f32;
    for influence in 0..4 {
        if !weights[influence].is_finite() || weights[influence] < 0.0 {
            return Err("VRM skin weight is invalid".to_string());
        }
        if weights[influence] > f32::EPSILON && usize::from(joints[influence]) >= joint_count {
            return Err("VRM vertex references a missing skin joint".to_string());
        }
        weight_sum += weights[influence];
    }
    if weight_sum <= f32::EPSILON {
        return Err("VRM skinned vertex must have at least one positive weight".to_string());
    }
    for weight in &mut weights {
        *weight /= weight_sum;
    }
    Ok((joints, weights))
}

fn finite_position(position: Vec3) -> Result<Vec3, String> {
    if position.is_finite() {
        Ok(position)
    } else {
        Err("VRM transformed vertex contains NaN or infinity".to_string())
    }
}

fn load_gltf_texture(
    gltf: &Gltf,
    texture_index: usize,
    blob: &[u8],
    total_decode_bytes: &mut u64,
    texture_cache: &mut HashMap<usize, usize>,
    textures: &mut Vec<CpuTexture>,
) -> Result<usize, String> {
    let texture = gltf
        .textures()
        .nth(texture_index)
        .ok_or_else(|| format!("VRM references missing texture {texture_index}"))?;
    let image = texture.source();
    if let Some(index) = texture_cache.get(&image.index()).copied() {
        return Ok(index);
    }
    if textures.len() >= MAX_TEXTURES {
        return Err(format!("VRM has more than {MAX_TEXTURES} textures"));
    }
    let decoded = decode_texture(image, blob, total_decode_bytes)?;
    let index = textures.len();
    textures.push(decoded);
    texture_cache.insert(texture.source().index(), index);
    Ok(index)
}

fn decode_texture(
    image: gltf::Image<'_>,
    blob: &[u8],
    total_decode_bytes: &mut u64,
) -> Result<CpuTexture, String> {
    let (view, mime_type) = match image.source() {
        gltf_image::Source::View { view, mime_type } => (view, mime_type),
        gltf_image::Source::Uri { .. } => {
            return Err("VRM external image URIs are not allowed".to_string());
        }
    };
    if !matches!(mime_type, "image/png" | "image/jpeg") {
        return Err(format!(
            "VRM embedded texture type {mime_type} is not supported"
        ));
    }
    let start = view.offset();
    let end = start
        .checked_add(view.length())
        .filter(|end| *end <= blob.len())
        .ok_or_else(|| "VRM embedded texture buffer view is truncated".to_string())?;
    let encoded = &blob[start..end];

    let mut reader = image::ImageReader::new(Cursor::new(encoded));
    reader.set_format(if mime_type == "image/png" {
        image::ImageFormat::Png
    } else {
        image::ImageFormat::Jpeg
    });
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_DIMENSION);
    limits.max_image_height = Some(MAX_TEXTURE_DIMENSION);
    limits.max_alloc = Some(MAX_TEXTURE_DECODE_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| format!("failed to decode VRM embedded texture: {error}"))?
        .to_rgba8();
    if decoded.width() == 0 || decoded.height() == 0 {
        return Err("VRM embedded texture must not be empty".to_string());
    }
    let decoded_bytes = u64::from(decoded.width())
        .checked_mul(u64::from(decoded.height()))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "VRM embedded texture size overflowed".to_string())?;
    *total_decode_bytes = total_decode_bytes
        .checked_add(decoded_bytes)
        .ok_or_else(|| "VRM texture allocation total overflowed".to_string())?;
    if *total_decode_bytes > MAX_TEXTURE_DECODE_BYTES {
        return Err(format!(
            "VRM decoded textures exceed the {} MiB total limit",
            MAX_TEXTURE_DECODE_BYTES / 1024 / 1024
        ));
    }

    Ok(CpuTexture {
        width: decoded.width(),
        height: decoded.height(),
        rgba: decoded.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder as _;

    #[test]
    fn valid_vrm_one_fixture_loads_mesh_and_embedded_texture() {
        let bytes = fixture_vrm_one(true);
        let scene = decode_bytes(&bytes, &[], 7).expect("decode generated VRM 1.0");

        assert_eq!(scene.scene_id, 7);
        assert_eq!(scene.version_label, "VRM 1.0");
        assert_eq!(scene.front_direction, 1.0);
        assert_eq!(scene.vertices.len(), 3);
        assert_eq!(scene.draws.len(), 1);
        assert_eq!(scene.textures.len(), 1);
        assert_eq!(scene.textures[0].rgba, [240, 120, 60, 255]);
    }

    #[test]
    fn embedded_gltf_skeletal_clip_updates_the_draw_pose() {
        let mut bin = fixture_bin();
        let image_end = bin.len();
        let time_offset = bin.len();
        for time in [0.0_f32, 1.0] {
            bin.extend_from_slice(&time.to_le_bytes());
        }
        let translation_offset = bin.len();
        for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        let mut root = fixture_root(true, bin.len());
        root["bufferViews"][2]["byteLength"] = serde_json::json!(image_end - 44);
        root["bufferViews"]
            .as_array_mut()
            .expect("buffer views")
            .extend([
                serde_json::json!({
                    "buffer": 0,
                    "byteOffset": time_offset,
                    "byteLength": 8
                }),
                serde_json::json!({
                    "buffer": 0,
                    "byteOffset": translation_offset,
                    "byteLength": 24
                }),
            ]);
        root["accessors"]
            .as_array_mut()
            .expect("accessors")
            .extend([
                serde_json::json!({
                    "bufferView": 3,
                    "componentType": 5126,
                    "count": 2,
                    "type": "SCALAR",
                    "min": [0.0],
                    "max": [1.0]
                }),
                serde_json::json!({
                    "bufferView": 4,
                    "componentType": 5126,
                    "count": 2,
                    "type": "VEC3"
                }),
            ]);
        root["animations"] = serde_json::json!([{
            "samplers": [{ "input": 2, "output": 3, "interpolation": "LINEAR" }],
            "channels": [{
                "sampler": 0,
                "target": { "node": 0, "path": "translation" }
            }]
        }]);

        let scene = decode_bytes(&assemble_glb(root, bin), &[], 9)
            .expect("decode VRM with embedded animation");
        let poses = scene.pose_matrices(0.5);
        let translation = Mat4::from_cols_array_2d(&poses[0][0]).transform_point3(Vec3::ZERO);

        assert!(!scene.animations.is_empty());
        assert_eq!(scene.animation_label(), Some("embedded glTF"));
        assert!((translation.x - 0.5).abs() < 1e-5);
    }

    #[test]
    fn valid_legacy_vrm_fixture_loads() {
        let bin = fixture_bin();
        let mut root = fixture_root(false, bin.len());
        let human_bones = REQUIRED_VRM_ZERO_HUMANOID_BONES
            .into_iter()
            .enumerate()
            .map(|(node, bone)| serde_json::json!({ "bone": bone, "node": node }))
            .collect::<Vec<_>>();
        root["nodes"] = Value::Array(humanoid_fixture_nodes(
            &REQUIRED_VRM_ZERO_HUMANOID_BONES,
            &REQUIRED_VRM_ZERO_HIERARCHY,
        ));
        root["scenes"] = serde_json::json!([{ "nodes": [0] }]);
        root["extensionsUsed"] = serde_json::json!(["VRM"]);
        root["extensionsRequired"] = serde_json::json!(["VRM"]);
        root["extensions"] = serde_json::json!({
            "VRM": {
                "specVersion": "0.0",
                "meta": {},
                "humanoid": { "humanBones": human_bones }
            }
        });

        let scene = decode_bytes(&assemble_glb(root, bin), &[], 8).expect("decode legacy VRM");
        assert_eq!(scene.version_label, "VRM 0.x");
        assert_eq!(scene.front_direction, -1.0);
        assert_eq!(scene.vertices.len(), 3);
    }

    #[test]
    fn legacy_expression_presets_map_to_vrm_one_runtime_names() {
        assert_eq!(canonical_legacy_expression_name("A"), "aa");
        assert_eq!(canonical_legacy_expression_name("Blink_L"), "blinkLeft");
        assert_eq!(canonical_legacy_expression_name("LookRight"), "lookRight");
        assert_eq!(canonical_legacy_expression_name("Joy"), "happy");
        assert_eq!(canonical_legacy_expression_name("customFace"), "customFace");
    }

    #[test]
    fn vrm_one_mtoon_material_parameters_are_loaded() {
        let bin = fixture_bin();
        let mut root = fixture_root(true, bin.len());
        root["extensionsUsed"] = serde_json::json!(["VRMC_vrm", "VRMC_materials_mtoon"]);
        root["extensionsRequired"] = serde_json::json!(["VRMC_vrm", "VRMC_materials_mtoon"]);
        root["materials"][0]["extensions"] = serde_json::json!({
            "VRMC_materials_mtoon": {
                "specVersion": "1.0",
                "shadeColorFactor": [0.2, 0.3, 0.4],
                "shadeMultiplyTexture": { "index": 0 },
                "shadingShiftFactor": -0.25,
                "shadingToonyFactor": 0.8,
                "parametricRimColorFactor": [0.1, 0.2, 0.3],
                "rimMultiplyTexture": { "index": 0 },
                "rimLightingMixFactor": 0.6,
                "matcapTexture": { "index": 0 },
                "matcapFactor": [0.7, 0.8, 0.9],
                "outlineWidthMode": "worldCoordinates",
                "outlineWidthFactor": 0.01,
                "outlineWidthMultiplyTexture": { "index": 0 },
                "uvAnimationScrollXSpeedFactor": 0.5
            }
        });
        root["materials"][0]["normalTexture"] = serde_json::json!({ "index": 0, "scale": 0.75 });

        let scene =
            decode_bytes(&assemble_glb(root, bin), &[], 10).expect("decode VRM MToon material");
        let mtoon = scene.materials[0].mtoon;

        assert!(mtoon.enabled);
        assert_eq!(mtoon.shade_color, [0.2, 0.3, 0.4]);
        assert_eq!(mtoon.shading_shift, -0.25);
        assert_eq!(mtoon.shading_toony, 0.8);
        assert_eq!(mtoon.outline_width, 0.01);
        assert_eq!(mtoon.uv_scroll, [0.5, 0.0]);
        assert_eq!(mtoon.shade_texture, Some(0));
        assert_eq!(mtoon.normal_texture, Some(0));
        assert_eq!(mtoon.matcap_texture, Some(0));
        assert_eq!(mtoon.rim_texture, Some(0));
        assert_eq!(mtoon.outline_width_texture, Some(0));
        assert!(!mtoon.outline_width_texture_uses_red);
        assert_eq!(mtoon.normal_scale, 0.75);
        assert_eq!(mtoon.matcap_color, [0.7, 0.8, 0.9]);
        assert_eq!(mtoon.rim_lighting_mix, 0.6);
        assert_eq!(scene.textures.len(), 1);
        assert!(scene.needs_continuous_update());
    }

    #[test]
    fn legacy_mtoon_outline_width_texture_uses_red_channel() {
        let root = serde_json::json!({
            "textures": [{ "source": 0 }],
            "extensions": {
                "VRM": {
                    "materialProperties": [{
                        "shader": "VRM/MToon",
                        "textureProperties": { "_OutlineWidthTexture": 0 }
                    }]
                }
            }
        });

        let material =
            decode_legacy_mtoon(&root, 0, [0.0; 3]).expect("decode legacy MToon material");

        assert_eq!(material.outline_width_texture, Some(0));
        assert!(material.outline_width_texture_uses_red);
    }

    #[test]
    fn vrm_expression_morph_is_loaded_and_evaluated() {
        let mut bin = fixture_bin();
        let image_length = bin.len() - 44;
        let morph_offset = bin.len();
        for value in [0.1_f32, 0.0, 0.0, 0.2, 0.0, 0.0, 0.3, 0.0, 0.0] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        let mut root = fixture_root(true, bin.len());
        root["bufferViews"][2]["byteLength"] = serde_json::json!(image_length);
        root["bufferViews"]
            .as_array_mut()
            .expect("buffer views")
            .push(serde_json::json!({
                "buffer": 0,
                "byteOffset": morph_offset,
                "byteLength": 36
            }));
        root["accessors"]
            .as_array_mut()
            .expect("accessors")
            .push(serde_json::json!({
                "bufferView": 3,
                "componentType": 5126,
                "count": 3,
                "type": "VEC3"
            }));
        root["meshes"][0]["primitives"][0]["targets"] = serde_json::json!([{ "POSITION": 2 }]);
        root["extensions"]["VRMC_vrm"]["expressions"] = serde_json::json!({
            "preset": {
                "happy": {
                    "morphTargetBinds": [{
                        "node": 0,
                        "index": 0,
                        "weight": 0.75
                    }]
                }
            }
        });

        let scene =
            decode_bytes(&assemble_glb(root, bin), &[], 12).expect("decode VRM expression morph");
        let morph = &scene.draws[0].morph_targets[0];
        let mut state = VrmRuntimeState::default();
        let frame = scene.evaluate_frame(
            FrameInput {
                time: 1.0,
                crossfade_seconds: 0.25,
                expression: "happy",
                look_yaw_degrees: 0.0,
                look_pitch_degrees: 0.0,
                spring_bone_enabled: true,
                look_at_enabled: true,
            },
            &mut state,
        );

        assert_eq!(morph.target_index, 0);
        assert_eq!(morph.deltas.len(), 3);
        assert!((morph.deltas[2].position[0] - 0.3).abs() < 1e-6);
        assert_eq!(frame.morph_weights.get(&(0, 0)), Some(&0.75));
    }

    #[test]
    fn look_at_constraints_and_spring_bones_are_loaded_together() {
        let bin = fixture_bin();
        let mut root = fixture_root(true, bin.len());
        root["extensionsUsed"] =
            serde_json::json!(["VRMC_vrm", "VRMC_node_constraint", "VRMC_springBone"]);
        root["extensionsRequired"] =
            serde_json::json!(["VRMC_vrm", "VRMC_node_constraint", "VRMC_springBone"]);
        root["extensions"]["VRMC_vrm"]["lookAt"] = serde_json::json!({
            "type": "expression",
            "rangeMapHorizontalOuter": {
                "inputMaxValue": 0.0,
                "outputScale": 1.0
            }
        });
        root["extensions"]["VRMC_springBone"] = serde_json::json!({
            "specVersion": "1.0",
            "colliders": [{
                "node": 0,
                "shape": { "sphere": { "radius": 0.2 } }
            }],
            "colliderGroups": [{ "name": "body", "colliders": [0] }],
            "springs": [{
                "center": 0,
                "joints": [
                    { "node": 0, "stiffness": 0.5, "dragForce": 0.25 },
                    { "node": 1 }
                ],
                "colliderGroups": [0]
            }]
        });
        root["nodes"][2]["extensions"] = serde_json::json!({
            "VRMC_node_constraint": {
                "specVersion": "1.0",
                "constraint": {
                    "rotation": { "source": 1, "weight": 0.5 }
                }
            }
        });

        let scene = decode_bytes(&assemble_glb(root, bin), &[], 13)
            .expect("decode lookAt, constraint, and SpringBone extensions");
        let look_at = scene.look_at.as_ref().expect("lookAt");

        assert_eq!(look_at.kind, LookAtKind::Expression);
        assert_eq!(look_at.horizontal_outer.input_max_degrees, 0.0);
        assert_eq!(scene.constraints.len(), 1);
        assert_eq!(scene.constraints[0].destination, 2);
        assert_eq!(scene.springs.len(), 1);
        assert_eq!(scene.springs[0].center, Some(0));
        assert_eq!(scene.springs[0].collider_indices, [0]);
        assert_eq!(scene.colliders.len(), 1);
    }

    #[test]
    fn spring_bone_collider_rejects_multiple_shapes() {
        let bin = fixture_bin();
        let mut root = fixture_root(true, bin.len());
        root["extensionsUsed"] = serde_json::json!(["VRMC_vrm", "VRMC_springBone"]);
        root["extensionsRequired"] = serde_json::json!(["VRMC_vrm", "VRMC_springBone"]);
        root["extensions"]["VRMC_springBone"] = serde_json::json!({
            "specVersion": "1.0",
            "colliders": [{
                "node": 0,
                "shape": {
                    "sphere": { "radius": 0.2 },
                    "capsule": { "radius": 0.2, "tail": [0.0, 0.1, 0.0] }
                }
            }]
        });

        let error = decode_bytes(&assemble_glb(root, bin), &[], 14)
            .expect_err("a collider shape must be exclusive");

        assert!(error.contains("exactly one sphere or capsule"));
    }

    #[test]
    fn aim_constraint_order_includes_hierarchy_dependencies() {
        let rest = NodeTransform {
            translation: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        let nodes = vec![
            CpuNode {
                rest,
                parent: None,
                active: true,
            },
            CpuNode {
                rest,
                parent: Some(0),
                active: true,
            },
            CpuNode {
                rest,
                parent: None,
                active: true,
            },
            CpuNode {
                rest,
                parent: None,
                active: true,
            },
        ];
        let aim = CpuNodeConstraint {
            destination: 2,
            source: 1,
            weight: 1.0,
            kind: ConstraintKind::Aim(Vec3::Z),
        };
        let ancestor_rotation = CpuNodeConstraint {
            destination: 0,
            source: 3,
            weight: 1.0,
            kind: ConstraintKind::Rotation,
        };

        let ordered = order_constraints(vec![aim, ancestor_rotation], &nodes)
            .expect("order hierarchy-dependent constraints");

        assert_eq!(ordered[0].destination, 0);
        assert_eq!(ordered[1].destination, 2);
    }

    #[test]
    fn hierarchy_constraint_cycle_is_rejected() {
        let rest = NodeTransform {
            translation: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        let nodes = vec![
            CpuNode {
                rest,
                parent: None,
                active: true,
            },
            CpuNode {
                rest,
                parent: Some(0),
                active: true,
            },
            CpuNode {
                rest,
                parent: None,
                active: true,
            },
        ];
        let error = order_constraints(
            vec![
                CpuNodeConstraint {
                    destination: 2,
                    source: 1,
                    weight: 1.0,
                    kind: ConstraintKind::Aim(Vec3::Z),
                },
                CpuNodeConstraint {
                    destination: 0,
                    source: 2,
                    weight: 1.0,
                    kind: ConstraintKind::Rotation,
                },
            ],
            &nodes,
        )
        .expect_err("hierarchy and source dependencies form a cycle");

        assert!(error.contains("circular dependency"));
    }

    #[test]
    fn external_vrma_humanoid_clip_is_retargeted_and_loaded() {
        let path = std::env::temp_dir().join(format!(
            "skid-monitor-vrma-{}-{}.vrma",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::write(&path, fixture_vrma()).expect("write VRMA fixture");

        let scene = decode_bytes(
            &fixture_vrm_one(true),
            &[path.to_str().expect("UTF-8 temp path").to_string()],
            11,
        )
        .expect("decode VRM with external VRMA");
        let poses = scene.pose_matrices(0.5);
        let rotated = Mat4::from_cols_array_2d(&poses[0][0]).transform_vector3(Vec3::X);

        assert_eq!(scene.animation_label(), Some("VRMA"));
        assert!((rotated.x - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        assert!((rotated.y - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn multiple_vrma_files_are_crossfaded_as_one_clip_sequence() {
        let prefix = format!(
            "skid-monitor-vrma-mix-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        );
        let first_path = std::env::temp_dir().join(format!("{prefix}-first.vrma"));
        let second_path = std::env::temp_dir().join(format!("{prefix}-second.vrma"));
        std::fs::write(
            &first_path,
            fixture_vrma_with_angle(std::f32::consts::FRAC_PI_2),
        )
        .expect("write first VRMA fixture");
        std::fs::write(
            &second_path,
            fixture_vrma_with_angle(-std::f32::consts::FRAC_PI_2),
        )
        .expect("write second VRMA fixture");

        let scene = decode_bytes(
            &fixture_vrm_one(true),
            &[
                first_path.to_string_lossy().into_owned(),
                second_path.to_string_lossy().into_owned(),
            ],
            14,
        )
        .expect("decode two VRMA clips");
        let poses = scene.pose_matrices(0.625);
        let rotated = Mat4::from_cols_array_2d(&poses[0][0]).transform_vector3(Vec3::X);
        let angle = rotated.y.atan2(rotated.x);

        assert_eq!(scene.animation_label(), Some("2 blended clips"));
        assert_eq!(scene.animations.len(), 2);
        assert!((angle - 33.75_f32.to_radians()).abs() < 1e-4);
        let _ = std::fs::remove_file(first_path);
        let _ = std::fs::remove_file(second_path);
    }

    #[test]
    fn ordinary_glb_renamed_to_vrm_is_rejected() {
        let bytes = fixture_vrm_one(false);
        let error = decode_bytes(&bytes, &[], 1).expect_err("VRM extension must be required");

        assert!(error.contains("VRM extension object is missing"));
    }

    #[test]
    fn malformed_glb_length_is_rejected_before_parsing() {
        let mut bytes = fixture_vrm_one(true);
        bytes[8..12].copy_from_slice(&12_u32.to_le_bytes());

        assert!(decode_bytes(&bytes, &[], 1).is_err());
    }

    #[test]
    fn unknown_required_extension_is_rejected() {
        let bin = fixture_bin();
        let mut root = fixture_root(true, bin.len());
        root["extensionsRequired"] = serde_json::json!(["EXT_execute_unknown_code"]);

        let error = decode_bytes(&assemble_glb(root, bin), &[], 1)
            .expect_err("unknown required extension must fail");
        assert!(error.contains("unsupported glTF extension"));
    }

    #[test]
    fn future_vrm_one_version_is_rejected() {
        let mut root = fixture_root(true, 44);
        root["extensions"]["VRMC_vrm"]["specVersion"] = Value::String("2.0".to_string());
        let bytes = assemble_glb(root, fixture_bin());

        let error = decode_bytes(&bytes, &[], 1).expect_err("future version must fail");
        assert!(error.contains("only VRM 1.0"));
    }

    #[test]
    fn vrm_one_missing_required_meta_is_rejected() {
        let bin = fixture_bin();
        let mut root = fixture_root(true, bin.len());
        root["extensions"]["VRMC_vrm"]["meta"] = serde_json::json!({
            "name": "Operator",
            "authors": [],
            "licenseUrl": "https://vrm.dev/licenses/1.0/"
        });

        let error = decode_bytes(&assemble_glb(root, bin), &[], 1)
            .expect_err("empty author list must fail VRM 1.0 metadata validation");
        assert!(error.contains("meta.authors"));
    }

    #[test]
    fn legacy_vrm_meta_fields_have_schema_types() {
        let bin = fixture_bin();
        let mut root = fixture_root(false, bin.len());
        let human_bones = REQUIRED_VRM_ZERO_HUMANOID_BONES
            .into_iter()
            .enumerate()
            .map(|(node, bone)| serde_json::json!({ "bone": bone, "node": node }))
            .collect::<Vec<_>>();
        root["extensions"] = serde_json::json!({
            "VRM": {
                "specVersion": "0.0",
                "meta": { "author": 42 },
                "humanoid": { "humanBones": human_bones }
            }
        });

        let error = decode_bytes(&assemble_glb(root, bin), &[], 1)
            .expect_err("legacy metadata with the wrong schema type must fail");
        assert!(error.contains("meta.author"));
    }

    #[test]
    fn legacy_vrm_requires_its_chest_and_neck_bones() {
        let human_bones = REQUIRED_VRM_ZERO_HUMANOID_BONES
            .into_iter()
            .filter(|bone| *bone != "chest")
            .enumerate()
            .map(|(node, bone)| serde_json::json!({ "bone": bone, "node": node }))
            .collect::<Vec<_>>();
        let vrm = serde_json::json!({
            "specVersion": "0.0",
            "meta": {},
            "humanoid": { "humanBones": human_bones }
        });

        let error = validate_vrm_zero(&vrm, &Value::Null, REQUIRED_VRM_ZERO_HUMANOID_BONES.len())
            .expect_err("legacy chest is a required humanoid bone");
        assert!(error.contains("chest"));
    }

    #[test]
    fn unsupported_legacy_vrm_version_is_rejected() {
        let vrm = serde_json::json!({ "specVersion": "0.1", "meta": {} });
        let error = validate_vrm_zero(&vrm, &Value::Null, REQUIRED_VRM_ZERO_HUMANOID_BONES.len())
            .expect_err("only the legacy 0.0 extension is supported");
        assert!(error.contains("version 0.1"));
    }

    #[test]
    fn mismatched_texture_accessor_is_rejected_before_collection() {
        let bin = fixture_bin();
        let mut root = fixture_root(true, bin.len());
        root["accessors"]
            .as_array_mut()
            .expect("fixture accessors")
            .push(serde_json::json!({
                "bufferView": 0,
                "componentType": 5126,
                "count": 4,
                "type": "VEC2"
            }));
        root["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = serde_json::json!(2);

        let error = decode_bytes(&assemble_glb(root, bin), &[], 1)
            .expect_err("oversized texture coordinate accessor must fail before collection");
        assert!(error.contains("texture coordinate count"));
    }

    #[test]
    fn detached_humanoid_bones_are_rejected() {
        let bin = fixture_bin();
        let mut root = fixture_root(true, bin.len());
        let nodes = (0..REQUIRED_VRM_ONE_HUMANOID_BONES.len())
            .map(|index| {
                if index == 0 {
                    serde_json::json!({ "mesh": 0 })
                } else {
                    serde_json::json!({})
                }
            })
            .collect::<Vec<_>>();
        root["nodes"] = Value::Array(nodes);
        root["scenes"] = serde_json::json!([{
            "nodes": (0..REQUIRED_VRM_ONE_HUMANOID_BONES.len()).collect::<Vec<_>>()
        }]);

        let error = decode_bytes(&assemble_glb(root, bin), &[], 1)
            .expect_err("detached required bones must not form a valid humanoid");
        assert!(error.contains("nearest required humanoid parent"));
    }

    #[test]
    fn interleaved_humanoid_branches_are_rejected() {
        let bin = fixture_bin();
        let mut root = fixture_root(true, bin.len());
        root["nodes"][0]["children"]
            .as_array_mut()
            .expect("hips children")
            .retain(|child| child.as_u64() != Some(1));
        root["nodes"][3]["children"]
            .as_array_mut()
            .expect("left upper leg children")
            .push(serde_json::json!(1));

        let error = decode_bytes(&assemble_glb(root, bin), &[], 1)
            .expect_err("spine nested under a leg must not form a valid humanoid");
        assert!(error.contains("spine"));
        assert!(error.contains("nearest required humanoid parent"));
    }

    #[test]
    fn blended_triangles_sort_back_to_front_for_each_vrm_axis() {
        let triangle = |z| [fixture_vertex(z), fixture_vertex(z), fixture_vertex(z)];
        let mut vrm_one = [triangle(2.0), triangle(-2.0)].concat();
        sort_blended_triangles(&mut vrm_one, &[2.0, -2.0], &mut []);
        assert_eq!(vrm_one[0].position[2], -2.0);

        let mut legacy = [triangle(-2.0), triangle(2.0)].concat();
        sort_blended_triangles(&mut legacy, &[2.0, -2.0], &mut []);
        assert_eq!(legacy[0].position[2], 2.0);
    }

    fn fixture_vrm_one(with_vrm_extension: bool) -> Vec<u8> {
        let bin = fixture_bin();
        assemble_glb(fixture_root(with_vrm_extension, bin.len()), bin)
    }

    fn fixture_vrma() -> Vec<u8> {
        fixture_vrma_with_angle(std::f32::consts::FRAC_PI_2)
    }

    fn fixture_vrma_with_angle(angle: f32) -> Vec<u8> {
        let mut human_bones = serde_json::Map::new();
        for (node, bone) in REQUIRED_VRM_ONE_HUMANOID_BONES.into_iter().enumerate() {
            human_bones.insert(bone.to_string(), serde_json::json!({ "node": node }));
        }
        let mut nodes = humanoid_fixture_nodes(
            &REQUIRED_VRM_ONE_HUMANOID_BONES,
            &REQUIRED_VRM_ONE_HIERARCHY,
        );
        nodes[0]
            .as_object_mut()
            .expect("VRMA root node")
            .remove("mesh");
        let mut bin = Vec::new();
        for time in [0.0_f32, 1.0] {
            bin.extend_from_slice(&time.to_le_bytes());
        }
        let half_turn = angle * 0.5;
        for value in [
            0.0_f32,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            half_turn.sin(),
            half_turn.cos(),
        ] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        let root = serde_json::json!({
            "asset": { "version": "2.0" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": nodes,
            "buffers": [{ "byteLength": bin.len() }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 8 },
                { "buffer": 0, "byteOffset": 8, "byteLength": 32 }
            ],
            "accessors": [
                {
                    "bufferView": 0,
                    "componentType": 5126,
                    "count": 2,
                    "type": "SCALAR",
                    "min": [0.0],
                    "max": [1.0]
                },
                {
                    "bufferView": 1,
                    "componentType": 5126,
                    "count": 2,
                    "type": "VEC4"
                }
            ],
            "animations": [{
                "samplers": [{ "input": 0, "output": 1, "interpolation": "LINEAR" }],
                "channels": [{
                    "sampler": 0,
                    "target": { "node": 0, "path": "rotation" }
                }]
            }],
            "extensionsUsed": ["VRMC_vrm_animation"],
            "extensionsRequired": ["VRMC_vrm_animation"],
            "extensions": {
                "VRMC_vrm_animation": {
                    "specVersion": "1.0",
                    "humanoid": { "humanBones": human_bones }
                }
            }
        });
        assemble_glb(root, bin)
    }

    fn fixture_root(with_vrm_extension: bool, bin_length: usize) -> Value {
        let mut human_bones = serde_json::Map::new();
        for (node, bone) in REQUIRED_VRM_ONE_HUMANOID_BONES.into_iter().enumerate() {
            human_bones.insert(bone.to_string(), serde_json::json!({ "node": node }));
        }
        let nodes = humanoid_fixture_nodes(
            &REQUIRED_VRM_ONE_HUMANOID_BONES,
            &REQUIRED_VRM_ONE_HIERARCHY,
        );
        let mut root = serde_json::json!({
            "asset": { "version": "2.0" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": nodes,
            "buffers": [{ "byteLength": bin_length }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 },
                { "buffer": 0, "byteOffset": 36, "byteLength": 6, "target": 34963 },
                { "buffer": 0, "byteOffset": 44, "byteLength": bin_length - 44 }
            ],
            "accessors": [
                {
                    "bufferView": 0,
                    "componentType": 5126,
                    "count": 3,
                    "type": "VEC3",
                    "min": [-0.5, 0.0, 0.0],
                    "max": [0.5, 1.0, 0.0]
                },
                { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }
            ],
            "images": [{ "bufferView": 2, "mimeType": "image/png" }],
            "textures": [{ "source": 0 }],
            "materials": [{
                "pbrMetallicRoughness": {
                    "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
                    "baseColorTexture": { "index": 0 }
                }
            }],
            "meshes": [{
                "primitives": [{
                    "attributes": { "POSITION": 0 },
                    "indices": 1,
                    "material": 0
                }]
            }]
        });
        if with_vrm_extension {
            root["extensionsUsed"] = serde_json::json!(["VRMC_vrm"]);
            root["extensionsRequired"] = serde_json::json!(["VRMC_vrm"]);
            root["extensions"] = serde_json::json!({
                "VRMC_vrm": {
                    "specVersion": "1.0",
                    "meta": {
                        "name": "Operator fixture",
                        "authors": ["monitor-cat tests"],
                        "licenseUrl": "https://vrm.dev/licenses/1.0/"
                    },
                    "humanoid": { "humanBones": human_bones }
                }
            });
        }
        root
    }

    fn humanoid_fixture_nodes(bones: &[&str], hierarchy: &[(&str, &str)]) -> Vec<Value> {
        let indices = bones
            .iter()
            .enumerate()
            .map(|(index, bone)| (*bone, index))
            .collect::<HashMap<_, _>>();
        let mut children = vec![Vec::new(); bones.len()];
        for (parent, child) in hierarchy {
            children[indices[parent]].push(indices[child]);
        }
        children
            .into_iter()
            .enumerate()
            .map(|(index, children)| {
                let mut node = serde_json::json!({});
                if index == 0 {
                    node["mesh"] = serde_json::json!(0);
                }
                if !children.is_empty() {
                    node["children"] = serde_json::json!(children);
                }
                node
            })
            .collect()
    }

    fn fixture_vertex(z: f32) -> VrmVertex {
        VrmVertex {
            position: [0.0, 0.0, z],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            joints: [0; 4],
            weights: [1.0, 0.0, 0.0, 0.0],
        }
    }

    fn fixture_bin() -> Vec<u8> {
        let mut bin = Vec::new();
        for value in [-0.5_f32, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        for index in [0_u16, 1, 2] {
            bin.extend_from_slice(&index.to_le_bytes());
        }
        while bin.len() < 44 {
            bin.push(0);
        }
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[240, 120, 60, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .expect("encode fixture texture");
        bin.extend_from_slice(&png);
        while bin.len() & 3 != 0 {
            bin.push(0);
        }
        bin
    }

    fn assemble_glb(root: Value, mut bin: Vec<u8>) -> Vec<u8> {
        let mut json = serde_json::to_vec(&root).expect("serialize fixture glTF");
        while json.len() & 3 != 0 {
            json.push(b' ');
        }
        while bin.len() & 3 != 0 {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut bytes = Vec::with_capacity(total);
        bytes.extend_from_slice(b"glTF");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&(total as u32).to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
        bytes.extend_from_slice(&json);
        bytes.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0x004E_4942_u32.to_le_bytes());
        bytes.extend_from_slice(&bin);
        bytes
    }
}
