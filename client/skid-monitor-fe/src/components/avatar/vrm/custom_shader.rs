use eframe::egui_wgpu::wgpu;
use std::io::Read as _;
use std::path::Path;
use std::sync::Arc;

const MAX_CUSTOM_SHADER_BYTES: u64 = 64 * 1024;
const CUSTOM_BEGIN: &str = "// SKID_CUSTOM_MATERIAL_BEGIN";
const CUSTOM_END: &str = "// SKID_CUSTOM_MATERIAL_END";
const REQUIRED_FUNCTION: &str = "skid_custom_material";

pub(super) struct LoadedCustomShader {
    pub(super) source: Arc<str>,
    pub(super) label: String,
}

pub(super) fn load(path: &str) -> Result<LoadedCustomShader, String> {
    let path = Path::new(path);
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wgsl"))
    {
        return Err("custom character shader must use a .wgsl extension".to_string());
    }
    let metadata = std::fs::metadata(path).map_err(|error| {
        format!(
            "failed to inspect custom WGSL shader {}: {error}",
            path.display()
        )
    })?;
    if metadata.len() > MAX_CUSTOM_SHADER_BYTES {
        return Err(format!(
            "custom WGSL shader exceeds the {} KiB limit",
            MAX_CUSTOM_SHADER_BYTES / 1024
        ));
    }
    let file = std::fs::File::open(path).map_err(|error| {
        format!(
            "failed to open custom WGSL shader {}: {error}",
            path.display()
        )
    })?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_CUSTOM_SHADER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            format!(
                "failed to read custom WGSL shader {}: {error}",
                path.display()
            )
        })?;
    if bytes.len() as u64 > MAX_CUSTOM_SHADER_BYTES {
        return Err(format!(
            "custom WGSL shader exceeds the {} KiB limit",
            MAX_CUSTOM_SHADER_BYTES / 1024
        ));
    }
    let custom = std::str::from_utf8(&bytes)
        .map_err(|_| "custom WGSL shader must be valid UTF-8".to_string())?;
    let source = compose(custom)?;
    let label = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("custom WGSL")
        .to_string();
    Ok(LoadedCustomShader {
        source: Arc::from(source),
        label,
    })
}

pub(super) fn compose(custom: &str) -> Result<String, String> {
    validate_custom_module(custom)?;
    let base = include_str!("shader.wgsl");
    let begin = base
        .find(CUSTOM_BEGIN)
        .ok_or_else(|| "built-in shader is missing its custom material begin marker".to_string())?;
    let end = base
        .find(CUSTOM_END)
        .ok_or_else(|| "built-in shader is missing its custom material end marker".to_string())?
        + CUSTOM_END.len();
    if begin >= end {
        return Err("built-in custom shader markers are out of order".to_string());
    }
    let mut source = String::with_capacity(base.len() + custom.len());
    source.push_str(&base[..begin]);
    source.push_str(CUSTOM_BEGIN);
    source.push('\n');
    source.push_str(custom.trim());
    source.push('\n');
    source.push_str(CUSTOM_END);
    source.push_str(&base[end..]);
    validate_module(&source, "customized VRM shader")?;
    Ok(source)
}

fn validate_custom_module(source: &str) -> Result<(), String> {
    if source.trim().is_empty() {
        return Err("custom WGSL shader source is empty".to_string());
    }
    for token in
        source.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
    {
        if matches!(token, "loop" | "while" | "for") {
            return Err("custom WGSL shader must not contain loops".to_string());
        }
    }
    let module = wgpu::naga::front::wgsl::parse_str(source)
        .map_err(|error| format!("custom WGSL parse failed: {error}"))?;
    if !module.entry_points.is_empty() {
        return Err("custom WGSL must not declare shader entry points".to_string());
    }
    if !module.global_variables.is_empty() {
        return Err("custom WGSL must not declare global resources or variables".to_string());
    }
    let required_count = module
        .functions
        .iter()
        .filter(|(_, function)| function.name.as_deref() == Some(REQUIRED_FUNCTION))
        .count();
    if required_count != 1 {
        return Err(format!(
            "custom WGSL must declare exactly one {REQUIRED_FUNCTION} function"
        ));
    }
    validate_parsed_module(&module, "custom WGSL")
}

fn validate_module(source: &str, label: &str) -> Result<(), String> {
    let module = wgpu::naga::front::wgsl::parse_str(source)
        .map_err(|error| format!("{label} parse failed: {error}"))?;
    validate_parsed_module(&module, label)
}

fn validate_parsed_module(module: &wgpu::naga::Module, label: &str) -> Result<(), String> {
    let mut validator = wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::empty(),
    );
    validator
        .validate(module)
        .map_err(|error| format!("{label} validation failed: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_CUSTOM: &str = r#"
fn skid_custom_material(
    color: vec4<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    world_position: vec3<f32>,
    time: f32,
) -> vec4<f32> {
    let pulse = 0.9 + 0.1 * sin(time + world_position.y + uv.x);
    return vec4<f32>(color.rgb * pulse * (0.8 + 0.2 * abs(normal.y)), color.a);
}
"#;

    #[test]
    fn custom_material_function_composes_with_the_builtin_shader() {
        let source = compose(VALID_CUSTOM).expect("valid custom shader");
        assert!(source.contains("let pulse ="));
        assert!(!source.contains("return color;\n// SKID_CUSTOM_MATERIAL_END"));
    }

    #[test]
    fn custom_shader_rejects_entry_points_and_loops() {
        assert!(compose("@compute @workgroup_size(1) fn skid_custom_material() {}").is_err());
        assert!(compose(&VALID_CUSTOM.replace("let pulse =", "loop {}\nlet pulse =")).is_err());
    }

    #[test]
    fn custom_shader_requires_the_stable_material_hook_signature() {
        let error =
            compose("fn skid_custom_material(color: vec4<f32>) -> vec4<f32> { return color; }")
                .expect_err("wrong signature must fail combined validation");
        assert!(error.contains("validation") || error.contains("parse"));
    }
}
