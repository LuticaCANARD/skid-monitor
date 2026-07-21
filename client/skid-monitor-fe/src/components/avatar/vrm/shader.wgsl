struct SceneUniform {
    center_and_scale: vec4<f32>,
    projection: vec4<f32>,
    animation: vec4<f32>,
};

struct MaterialUniform {
    base_color: vec4<f32>,
    alpha: vec4<f32>,
    shade: vec4<f32>,
    toon: vec4<f32>,
    rim: vec4<f32>,
    emissive_outline: vec4<f32>,
    outline: vec4<f32>,
    uv_animation: vec4<f32>,
    matcap_normal: vec4<f32>,
    texture_flags: vec4<f32>,
    texture_flags_2: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> scene: SceneUniform;

@group(1) @binding(0)
var base_color_texture: texture_2d<f32>;

@group(1) @binding(1)
var base_color_sampler: sampler;

@group(1) @binding(2)
var<uniform> material: MaterialUniform;

@group(1) @binding(3)
var shade_texture: texture_2d<f32>;

@group(1) @binding(4)
var normal_texture: texture_2d<f32>;

@group(1) @binding(5)
var matcap_texture: texture_2d<f32>;

@group(1) @binding(6)
var rim_texture: texture_2d<f32>;

@group(1) @binding(7)
var outline_width_texture: texture_2d<f32>;

@group(2) @binding(0)
var<storage, read> pose_matrices: array<mat4x4<f32>>;

struct MorphDelta {
    position: vec4<f32>,
    normal: vec4<f32>,
    tangent: vec4<f32>,
};

struct DrawUniform {
    vertex_start: u32,
    vertex_count: u32,
    morph_count: u32,
    padding: u32,
};

@group(2) @binding(1)
var<storage, read> morph_deltas: array<MorphDelta>;

@group(2) @binding(2)
var<storage, read> morph_weights: array<f32>;

@group(2) @binding(3)
var<uniform> draw_uniform: DrawUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) tangent: vec4<f32>,
};

fn skin_matrix(input: VertexInput) -> mat4x4<f32> {
    return pose_matrices[input.joints.x] * input.weights.x
        + pose_matrices[input.joints.y] * input.weights.y
        + pose_matrices[input.joints.z] * input.weights.z
        + pose_matrices[input.joints.w] * input.weights.w;
}

fn project_position(position: vec3<f32>) -> vec4<f32> {
    let centered = position - scene.center_and_scale.xyz;
    let scale = scene.center_and_scale.w;
    let aspect = max(scene.projection.x, 0.01);
    let depth_span = max(scene.projection.z, 0.0001);
    let oriented_z = position.z * scene.projection.w;
    let depth = 0.05 + 0.9 * (scene.projection.y - oriented_z) / depth_span;
    return vec4<f32>(
        centered.x * scene.projection.w * scale / aspect,
        centered.y * scale,
        clamp(depth, 0.0, 1.0),
        1.0,
    );
}

fn animated_uv(uv: vec2<f32>) -> vec2<f32> {
    let seconds = scene.animation.x;
    let angle = material.uv_animation.z * seconds;
    let centered = uv - vec2<f32>(0.5);
    let rotated = vec2<f32>(
        centered.x * cos(angle) - centered.y * sin(angle),
        centered.x * sin(angle) + centered.y * cos(angle),
    );
    return rotated + vec2<f32>(0.5) + material.uv_animation.xy * seconds;
}

fn make_vertex(input: VertexInput, vertex_index: u32, outline: bool) -> VertexOutput {
    var local_position = input.position;
    var local_normal = input.normal;
    var local_tangent = input.tangent.xyz;
    let local_vertex = vertex_index - draw_uniform.vertex_start;
    for (var morph_index = 0u; morph_index < draw_uniform.morph_count; morph_index += 1u) {
        let delta_index = morph_index * draw_uniform.vertex_count + local_vertex;
        let weight = morph_weights[morph_index];
        let delta = morph_deltas[delta_index];
        local_position += delta.position.xyz * weight;
        local_normal += delta.normal.xyz * weight;
        local_tangent += delta.tangent.xyz * weight;
    }
    let skin = skin_matrix(input);
    var world_position = (skin * vec4<f32>(local_position, 1.0)).xyz;
    let world_normal = normalize((skin * vec4<f32>(local_normal, 0.0)).xyz);
    let world_tangent = normalize((skin * vec4<f32>(local_tangent, 0.0)).xyz);
    if outline {
        let outline_texel = textureSampleLevel(
            outline_width_texture,
            base_color_sampler,
            animated_uv(input.uv),
            0.0,
        );
        let outline_sample = mix(outline_texel.g, outline_texel.r, material.texture_flags_2.z);
        let outline_mask = mix(1.0, outline_sample, material.texture_flags_2.x);
        let width = min(max(material.emissive_outline.w * outline_mask, 0.0), 0.05);
        world_position += world_normal * width;
    }

    var output: VertexOutput;
    output.clip_position = project_position(world_position);
    output.normal = world_normal;
    output.uv = animated_uv(input.uv);
    output.world_position = world_position;
    output.tangent = vec4<f32>(world_tangent, input.tangent.w);
    return output;
}

@vertex
fn vs_main(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    return make_vertex(input, vertex_index, false);
}

@vertex
fn vs_outline(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    return make_vertex(input, vertex_index, true);
}

fn linearstep(low: f32, high: f32, value: f32) -> f32 {
    if high - low <= 0.00001 {
        return select(0.0, 1.0, value >= high);
    }
    return clamp((value - low) / (high - low), 0.0, 1.0);
}

fn surface_normal(input: VertexOutput) -> vec3<f32> {
    let geometric_normal = normalize(input.normal);
    if material.texture_flags.y < 0.5 {
        return geometric_normal;
    }
    var sampled_normal = textureSample(normal_texture, base_color_sampler, input.uv).xyz
        * vec3<f32>(2.0)
        - vec3<f32>(1.0);
    sampled_normal.x *= material.matcap_normal.w;
    sampled_normal.y *= material.matcap_normal.w;
    sampled_normal = normalize(sampled_normal);
    let tangent = normalize(input.tangent.xyz);
    let bitangent = normalize(cross(geometric_normal, tangent)) * input.tangent.w;
    return normalize(
        tangent * sampled_normal.x
            + bitangent * sampled_normal.y
            + geometric_normal * sampled_normal.z,
    );
}

fn shaded_color(input: VertexOutput) -> vec4<f32> {
    let sampled = textureSample(base_color_texture, base_color_sampler, input.uv);
    let base = sampled * material.base_color;
    let normal = surface_normal(input);
    let light_direction = normalize(vec3<f32>(0.35, 0.75, 0.55));

    if material.alpha.z > 0.5 {
        let lambert = dot(normal, light_direction);
        let shifted = lambert + material.shade.w;
        let toony = clamp(material.toon.x, 0.0, 1.0);
        let lighting = linearstep(-1.0 + toony, 1.0 - toony, shifted);
        let shade_sample = textureSample(shade_texture, base_color_sampler, input.uv).rgb;
        let shade_multiplier = mix(vec3<f32>(1.0), shade_sample, material.texture_flags.x);
        let shade_term = material.shade.rgb * shade_multiplier;
        var color = mix(shade_term, base.rgb, lighting);

        let ambient_strength = mix(0.12, 0.28, clamp(material.toon.y, 0.0, 1.0));
        color = color * (1.0 - ambient_strength) + base.rgb * ambient_strength;
        let view_direction = normalize(vec3<f32>(0.0, 0.0, scene.projection.w));
        let rim_base = clamp(
            1.0 - dot(normal, view_direction) + material.toon.w,
            0.0,
            1.0,
        );
        let rim_power = max(material.toon.z, 0.0001);
        let world_view_x = normalize(vec3<f32>(view_direction.z, 0.0, -view_direction.x));
        let world_view_y = cross(view_direction, world_view_x);
        let matcap_uv = vec2<f32>(
            dot(world_view_x, normal),
            dot(world_view_y, normal),
        ) * 0.495 + vec2<f32>(0.5);
        let matcap_sample = textureSample(matcap_texture, base_color_sampler, matcap_uv).rgb;
        let matcap = matcap_sample * material.matcap_normal.rgb * material.texture_flags.z;
        let parametric_rim = pow(rim_base, rim_power) * material.rim.rgb;
        let rim_sample = textureSample(rim_texture, base_color_sampler, input.uv).rgb;
        let rim_multiplier = mix(vec3<f32>(1.0), rim_sample, material.texture_flags.w);
        let rim_lighting = mix(
            vec3<f32>(1.0),
            vec3<f32>(lighting),
            clamp(material.texture_flags_2.y, 0.0, 1.0),
        );
        color += (matcap + parametric_rim) * rim_multiplier * rim_lighting;
        color += material.emissive_outline.rgb;
        return vec4<f32>(max(color, vec3<f32>(0.0)), base.a);
    }

    let diffuse = 0.55 + 0.45 * abs(dot(normal, light_direction));
    return vec4<f32>(base.rgb * diffuse, base.a);
}

fn outline_color(input: VertexOutput) -> vec4<f32> {
    let sampled = textureSample(base_color_texture, base_color_sampler, input.uv);
    let light_direction = normalize(vec3<f32>(0.35, 0.75, 0.55));
    let lighting = 0.55 + 0.45 * abs(dot(normalize(input.normal), light_direction));
    let mix_factor = clamp(material.outline.w, 0.0, 1.0);
    return vec4<f32>(
        material.outline.rgb * mix(1.0, lighting, mix_factor),
        sampled.a * material.base_color.a,
    );
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

@fragment
fn fs_outline_linear_framebuffer(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = outline_color(input);
    if material.alpha.y > 0.5 && color.a < material.alpha.x {
        discard;
    }
    return color;
}

@fragment
fn fs_outline_gamma_framebuffer(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = outline_color(input);
    if material.alpha.y > 0.5 && color.a < material.alpha.x {
        discard;
    }
    return vec4<f32>(gamma_from_linear_rgb(color.rgb), color.a);
}
