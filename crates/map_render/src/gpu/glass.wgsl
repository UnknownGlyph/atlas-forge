struct Material {
    bounds: vec4<f32>,
    view_size: vec2<f32>,
    pixels_per_point: f32,
    dark: f32,
};

@group(0) @binding(0) var maps: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<uniform> material: Material;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) map_uv: vec2<f32>,
    @location(1) panel_uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    let corners = array(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0)
    );
    let uv = corners[index];
    let at = mix(material.bounds.xy, material.bounds.zw, uv);
    return Vertex(vec4(2.0 * at.x - 1.0, 1.0 - 2.0 * at.y, 0.0, 1.0), at, uv);
}

fn sample_map(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(maps, linear_sampler, uv, 0.0).rgb;
}

@fragment
fn fs_main(in: Vertex) -> @location(0) vec4<f32> {
    let pixel = material.pixels_per_point;
    let size = (material.bounds.zw - material.bounds.xy) * material.view_size;
    let radius = min(19.0 * pixel, min(size.x, size.y) * 0.49);
    let p = abs((in.panel_uv - vec2(0.5)) * size) - size * 0.5 + radius;
    let edge = length(max(p, vec2(0.0))) + min(max(p.x, p.y), 0.0) - radius;
    let opacity = 1.0 - smoothstep(-0.8, 0.5, edge);
    if opacity <= 0.0 { discard; }

    let step = vec2(10.0 * pixel) / material.view_size;
    let uv = in.map_uv;
    let soft = (
        4.0 * sample_map(uv)
        + 2.0 * sample_map(uv + vec2(step.x, 0.0))
        + 2.0 * sample_map(uv - vec2(step.x, 0.0))
        + 2.0 * sample_map(uv + vec2(0.0, step.y))
        + 2.0 * sample_map(uv - vec2(0.0, step.y))
        + sample_map(uv + step)
        + sample_map(uv - step)
        + sample_map(uv + vec2(-step.x, step.y))
        + sample_map(uv + vec2(step.x, -step.y))
    ) / 16.0;
    let is_dark = material.dark > 0.5;
    let light_tint = mix(vec3(0.84, 0.91, 0.98), vec3(1.0, 0.995, 0.985), 1.0 - in.panel_uv.y);
    let dark_tint = mix(vec3(0.09, 0.11, 0.15), vec3(0.16, 0.19, 0.25), 1.0 - in.panel_uv.y);
    let tint = select(light_tint, dark_tint, is_dark);
    let veil = select(0.63, 0.72, is_dark);
    let shine = select(0.68, 0.2, is_dark);
    let rim = smoothstep(-2.2 * pixel, -0.3 * pixel, edge) * (1.0 - in.panel_uv.y * 0.48);
    let rgb = mix(mix(soft, tint, veil), vec3(1.0), rim * shine);
    return vec4(rgb * opacity, opacity);
}
