fn skid_custom_material(
    color: vec4<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    world_position: vec3<f32>,
    time: f32,
) -> vec4<f32> {
    let scan = 0.9 + 0.1 * sin(time * 2.0 + world_position.y * 12.0 + uv.x * 3.0);
    let facing = 0.8 + 0.2 * abs(normal.y);
    return vec4<f32>(color.rgb * scan * facing, color.a);
}
