@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> step: vec4<f32>;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Vertex;
    out.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

fn blur(uv: vec2<f32>) -> vec3<f32> {
    var value = textureSample(scene, scene_sampler, uv).rgb * 0.204164;
    let weights = array<f32, 4>(0.180174, 0.123832, 0.066282, 0.027630);
    for (var i = 0u; i < 4u; i += 1u) {
        let offset = step.xy * f32(i + 1u);
        value += (textureSample(scene, scene_sampler, uv + offset).rgb
            + textureSample(scene, scene_sampler, uv - offset).rgb) * weights[i];
    }
    return value;
}

fn gamma(rgb: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(rgb, vec3<f32>(1.0 / 2.4)) - 0.055, rgb * 12.92, rgb <= vec3<f32>(0.0031308));
}

fn glass(rgb: vec3<f32>, light: bool) -> vec4<f32> {
    let transmitted = select(rgb * 0.55, rgb, light);
    return vec4<f32>(gamma(transmitted), 1.0);
}

@fragment
fn horizontal(in: Vertex) -> @location(0) vec4<f32> {
    return vec4<f32>(blur(in.uv), 1.0);
}

@fragment
fn dark(in: Vertex) -> @location(0) vec4<f32> {
    return glass(blur(in.uv), false);
}

@fragment
fn light(in: Vertex) -> @location(0) vec4<f32> {
    return glass(blur(in.uv), true);
}

@fragment
fn dark_sharp(in: Vertex) -> @location(0) vec4<f32> {
    return glass(textureSample(scene, scene_sampler, in.uv).rgb, false);
}

@fragment
fn light_sharp(in: Vertex) -> @location(0) vec4<f32> {
    return glass(textureSample(scene, scene_sampler, in.uv).rgb, true);
}
