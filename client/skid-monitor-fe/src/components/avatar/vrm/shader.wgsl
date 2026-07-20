struct SceneUniform {
    center_and_scale: vec4<f32>,
    projection: vec4<f32>,
};

struct MaterialUniform {
    base_color: vec4<f32>,
    alpha: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> scene: SceneUniform;

@group(1) @binding(0)
var base_color_texture: texture_2d<f32>;

@group(1) @binding(1)
var base_color_sampler: sampler;

@group(1) @binding(2)
var<uniform> material: MaterialUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let centered = input.position - scene.center_and_scale.xyz;
    let scale = scene.center_and_scale.w;
    let aspect = max(scene.projection.x, 0.01);
    let depth_span = max(scene.projection.z, 0.0001);
    let oriented_z = input.position.z * scene.projection.w;
    let depth = 0.05 + 0.9 * (scene.projection.y - oriented_z) / depth_span;

    var output: VertexOutput;
    output.clip_position = vec4<f32>(
        centered.x * scene.projection.w * scale / aspect,
        centered.y * scale,
        clamp(depth, 0.0, 1.0),
        1.0,
    );
    output.normal = normalize(input.normal);
    output.uv = input.uv;
    return output;
}

fn shaded_color(input: VertexOutput) -> vec4<f32> {
    let sampled = textureSample(base_color_texture, base_color_sampler, input.uv);
    let color = sampled * material.base_color;
    let light_direction = normalize(vec3<f32>(0.35, 0.75, 0.55));
    let diffuse = 0.55 + 0.45 * abs(dot(normalize(input.normal), light_direction));
    return vec4<f32>(color.rgb * diffuse, color.a);
}

fn gamma_from_linear_rgb(rgb: vec3<f32>) -> vec3<f32> {
    let safe_rgb = max(rgb, vec3<f32>(0.0));
    let cutoff = safe_rgb < vec3<f32>(0.0031308);
    let lower = safe_rgb * vec3<f32>(12.92);
    let higher = vec3<f32>(1.055) * pow(safe_rgb, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(higher, lower, cutoff);
}

@fragment
fn fs_main_linear_framebuffer(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = shaded_color(input);
    if material.alpha.y > 0.5 && color.a < material.alpha.x {
        discard;
    }
    return color;
}

@fragment
fn fs_main_gamma_framebuffer(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = shaded_color(input);
    if material.alpha.y > 0.5 && color.a < material.alpha.x {
        discard;
    }
    return vec4<f32>(gamma_from_linear_rgb(color.rgb), color.a);
}
