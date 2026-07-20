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
const MAX_SKIN_JOINTS: usize = 256;
const MAX_TEXTURES: usize = 64;
const MAX_TEXTURE_DIMENSION: u32 = 4096;
const MAX_TEXTURE_DECODE_BYTES: u64 = 128 * 1024 * 1024;

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

const SUPPORTED_REQUIRED_EXTENSIONS: [&str; 2] = ["VRMC_vrm", "VRM"];

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(super) struct VrmVertex {
    pub(super) position: [f32; 3],
    pub(super) normal: [f32; 3],
    pub(super) uv: [f32; 2],
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
}

#[derive(Clone, Debug)]
pub(super) struct CpuDraw {
    pub(super) vertices: Range<u32>,
    pub(super) material_index: usize,
    pub(super) center_z: f32,
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
}

pub(super) fn decode(path: &str, scene_id: u64) -> Result<CpuVrmScene, String> {
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

    decode_bytes(&bytes, scene_id)
}

fn decode_bytes(bytes: &[u8], scene_id: u64) -> Result<CpuVrmScene, String> {
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
    let worlds = collect_world_transforms(&gltf)?;
    build_scene(&gltf, blob, &worlds, scene_id, version_label)
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
    let Some(required) = root.get("extensionsRequired") else {
        return Ok(());
    };
    let required = required
        .as_array()
        .ok_or_else(|| "VRM extensionsRequired must be an array".to_string())?;
    for extension in required {
        let extension = extension
            .as_str()
            .ok_or_else(|| "VRM required extension name must be a string".to_string())?;
        if !SUPPORTED_REQUIRED_EXTENSIONS.contains(&extension) {
            return Err(format!(
                "VRM requires unsupported glTF extension {extension}"
            ));
        }
    }
    Ok(())
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

fn validate_accessor_iteration(accessor: &gltf::Accessor<'_>, label: &str) -> Result<(), String> {
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

fn collect_world_transforms(gltf: &Gltf) -> Result<Vec<Option<Mat4>>, String> {
    let mut worlds = vec![None; gltf.nodes().len()];
    let scene = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .ok_or_else(|| "VRM scene is missing".to_string())?;
    for node in scene.nodes() {
        walk_node(node, Mat4::IDENTITY, 0, &mut worlds)?;
    }
    Ok(worlds)
}

fn walk_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    depth: usize,
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
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent * local;
    if !world.to_cols_array().iter().all(|value| value.is_finite()) {
        return Err(format!(
            "VRM node {} has a non-finite transform",
            node.index()
        ));
    }
    worlds[node.index()] = Some(world);
    for child in node.children() {
        walk_node(child, world, depth + 1, worlds)?;
    }
    Ok(())
}

fn build_scene(
    gltf: &Gltf,
    blob: &[u8],
    worlds: &[Option<Mat4>],
    scene_id: u64,
    version_label: &'static str,
) -> Result<CpuVrmScene, String> {
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
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for node in gltf.nodes() {
        let Some(mesh) = node.mesh() else {
            continue;
        };
        let Some(mesh_world) = worlds[node.index()] else {
            continue;
        };
        let skin_matrices = skin_matrices(node.clone(), worlds, blob)?;

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
            let tex_coords = if base_texture.is_some() {
                match primitive.get(&Semantic::TexCoords(tex_coord_set)) {
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
                }
            } else {
                None
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

            let texture_index = if let Some(texture) = base_texture {
                let image = texture.texture().source();
                if let Some(index) = texture_cache.get(&image.index()).copied() {
                    Some(index)
                } else {
                    if textures.len() >= MAX_TEXTURES {
                        return Err(format!("VRM has more than {MAX_TEXTURES} textures"));
                    }
                    let decoded = decode_texture(image, blob, &mut texture_decode_bytes)?;
                    let index = textures.len();
                    textures.push(decoded);
                    texture_cache.insert(texture.texture().source().index(), index);
                    Some(index)
                }
            } else {
                None
            };
            let alpha_mode = match material.alpha_mode() {
                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                gltf::material::AlphaMode::Mask => AlphaMode::Mask,
                gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            };
            let material_index = materials.len();
            materials.push(CpuMaterial {
                base_color: pbr.base_color_factor(),
                texture_index,
                alpha_mode,
                alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
            });

            let start_index = vertices.len();
            let start = u32::try_from(start_index)
                .map_err(|_| "VRM vertex buffer is too large".to_string())?;
            for triangle in indices.chunks_exact(3) {
                let mut points = [Vec3::ZERO; 3];
                let mut uvs = [[0.0_f32; 2]; 3];
                for corner in 0..3 {
                    let index = usize::try_from(triangle[corner])
                        .map_err(|_| "VRM vertex index is too large".to_string())?;
                    let position = positions
                        .get(index)
                        .ok_or_else(|| format!("VRM index {index} is outside POSITION data"))?;
                    points[corner] = transform_position(
                        Vec3::from_array(*position),
                        index,
                        mesh_world,
                        skin_matrices.as_deref(),
                        joints.as_deref(),
                        weights.as_deref(),
                    )?;
                    if let Some(coords) = &tex_coords {
                        uvs[corner] = coords[index];
                    }
                }

                let face = (points[1] - points[0]).cross(points[2] - points[0]);
                if !face.is_finite() || face.length_squared() <= f32::EPSILON {
                    continue;
                }
                let normal = face.normalize().to_array();
                for corner in 0..3 {
                    min = min.min(points[corner]);
                    max = max.max(points[corner]);
                    vertices.push(VrmVertex {
                        position: points[corner].to_array(),
                        normal,
                        uv: uvs[corner],
                    });
                }
                if vertices.len() > MAX_OUTPUT_VERTICES {
                    return Err(format!(
                        "VRM expanded vertex count exceeds {MAX_OUTPUT_VERTICES}"
                    ));
                }
            }
            let end = u32::try_from(vertices.len())
                .map_err(|_| "VRM vertex buffer is too large".to_string())?;
            if end > start {
                if alpha_mode == AlphaMode::Blend {
                    sort_blended_triangles(&mut vertices[start_index..], front_direction);
                }
                let (draw_min_z, draw_max_z) = vertices[start_index..]
                    .iter()
                    .map(|vertex| vertex.position[2])
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), z| {
                        (min.min(z), max.max(z))
                    });
                draws.push(CpuDraw {
                    vertices: start..end,
                    material_index,
                    center_z: (draw_min_z + draw_max_z) * 0.5,
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
    })
}

fn sort_blended_triangles(vertices: &mut [VrmVertex], front_direction: f32) {
    let mut triangles = vertices
        .chunks_exact(3)
        .map(|triangle| [triangle[0], triangle[1], triangle[2]])
        .collect::<Vec<_>>();
    triangles.sort_by(|left, right| {
        triangle_depth(left, front_direction).total_cmp(&triangle_depth(right, front_direction))
    });
    for (target, triangle) in vertices.chunks_exact_mut(3).zip(triangles) {
        target.copy_from_slice(&triangle);
    }
}

fn triangle_depth(triangle: &[VrmVertex; 3], front_direction: f32) -> f32 {
    triangle
        .iter()
        .map(|vertex| vertex.position[2])
        .sum::<f32>()
        * (front_direction / 3.0)
}

fn skin_matrices(
    node: gltf::Node<'_>,
    worlds: &[Option<Mat4>],
    blob: &[u8],
) -> Result<Option<Vec<Mat4>>, String> {
    let Some(skin) = node.skin() else {
        return Ok(None);
    };
    let joints: Vec<_> = skin.joints().collect();
    if joints.is_empty() || joints.len() > MAX_SKIN_JOINTS {
        return Err(format!(
            "VRM skin joint count must be between 1 and {MAX_SKIN_JOINTS}"
        ));
    }
    if let Some(accessor) = skin.inverse_bind_matrices() {
        validate_accessor_iteration(&accessor, "inverse bind matrix")?;
        if accessor.count() != joints.len() {
            return Err("VRM inverse bind matrix count does not match skin joints".to_string());
        }
        if accessor.data_type() != DataType::F32
            || accessor.dimensions() != Dimensions::Mat4
            || accessor.normalized()
        {
            return Err("VRM inverse bind accessor must be non-normalized F32 MAT4".to_string());
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

    joints
        .into_iter()
        .zip(inverse_bind)
        .map(|(joint, inverse_bind)| {
            let world = worlds
                .get(joint.index())
                .and_then(|matrix| *matrix)
                .ok_or_else(|| {
                    format!(
                        "VRM skin joint node {} is outside the active scene",
                        joint.index()
                    )
                })?;
            let matrix = world * inverse_bind;
            if !matrix.to_cols_array().iter().all(|value| value.is_finite()) {
                return Err("VRM skin matrix contains NaN or infinity".to_string());
            }
            Ok(matrix)
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
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

fn finite_position(position: Vec3) -> Result<Vec3, String> {
    if position.is_finite() {
        Ok(position)
    } else {
        Err("VRM transformed vertex contains NaN or infinity".to_string())
    }
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
        let scene = decode_bytes(&bytes, 7).expect("decode generated VRM 1.0");

        assert_eq!(scene.scene_id, 7);
        assert_eq!(scene.version_label, "VRM 1.0");
        assert_eq!(scene.front_direction, 1.0);
        assert_eq!(scene.vertices.len(), 3);
        assert_eq!(scene.draws.len(), 1);
        assert_eq!(scene.textures.len(), 1);
        assert_eq!(scene.textures[0].rgba, [240, 120, 60, 255]);
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

        let scene = decode_bytes(&assemble_glb(root, bin), 8).expect("decode legacy VRM");
        assert_eq!(scene.version_label, "VRM 0.x");
        assert_eq!(scene.front_direction, -1.0);
        assert_eq!(scene.vertices.len(), 3);
    }

    #[test]
    fn ordinary_glb_renamed_to_vrm_is_rejected() {
        let bytes = fixture_vrm_one(false);
        let error = decode_bytes(&bytes, 1).expect_err("VRM extension must be required");

        assert!(error.contains("VRM extension object is missing"));
    }

    #[test]
    fn malformed_glb_length_is_rejected_before_parsing() {
        let mut bytes = fixture_vrm_one(true);
        bytes[8..12].copy_from_slice(&12_u32.to_le_bytes());

        assert!(decode_bytes(&bytes, 1).is_err());
    }

    #[test]
    fn unknown_required_extension_is_rejected() {
        let bin = fixture_bin();
        let mut root = fixture_root(true, bin.len());
        root["extensionsRequired"] = serde_json::json!(["EXT_execute_unknown_code"]);

        let error = decode_bytes(&assemble_glb(root, bin), 1)
            .expect_err("unknown required extension must fail");
        assert!(error.contains("unsupported glTF extension"));
    }

    #[test]
    fn future_vrm_one_version_is_rejected() {
        let mut root = fixture_root(true, 44);
        root["extensions"]["VRMC_vrm"]["specVersion"] = Value::String("2.0".to_string());
        let bytes = assemble_glb(root, fixture_bin());

        let error = decode_bytes(&bytes, 1).expect_err("future version must fail");
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

        let error = decode_bytes(&assemble_glb(root, bin), 1)
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

        let error = decode_bytes(&assemble_glb(root, bin), 1)
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

        let error = decode_bytes(&assemble_glb(root, bin), 1)
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

        let error = decode_bytes(&assemble_glb(root, bin), 1)
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

        let error = decode_bytes(&assemble_glb(root, bin), 1)
            .expect_err("spine nested under a leg must not form a valid humanoid");
        assert!(error.contains("spine"));
        assert!(error.contains("nearest required humanoid parent"));
    }

    #[test]
    fn blended_triangles_sort_back_to_front_for_each_vrm_axis() {
        let triangle = |z| [fixture_vertex(z), fixture_vertex(z), fixture_vertex(z)];
        let mut vrm_one = [triangle(2.0), triangle(-2.0)].concat();
        sort_blended_triangles(&mut vrm_one, 1.0);
        assert_eq!(vrm_one[0].position[2], -2.0);

        let mut legacy = [triangle(-2.0), triangle(2.0)].concat();
        sort_blended_triangles(&mut legacy, -1.0);
        assert_eq!(legacy[0].position[2], 2.0);
    }

    fn fixture_vrm_one(with_vrm_extension: bool) -> Vec<u8> {
        let bin = fixture_bin();
        assemble_glb(fixture_root(with_vrm_extension, bin.len()), bin)
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
