#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
    utils::coords_to_viewport_uv,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var clouds_render_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var clouds_render_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var sky_texture: texture_2d<f32>;
// Direction towards the sun, with the disc's angular radius in `w`.
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var<uniform> sun: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var<uniform> sun_radiance: vec4<f32>;

struct SkyOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

// The sun's disc along a view ray. It is drawn here rather than into the
// half-resolution sky so its edge stays sharp, darkened towards the limb as
// the real one is, and cut off below the horizon.
fn sun_disc(ray: vec3<f32>) -> vec3<f32> {
    let angle = acos(clamp(dot(ray, sun.xyz), -1.0, 1.0));
    let soft = max(fwidth(angle), 1e-5);
    let disc = 1.0 - smoothstep(sun.w - soft, sun.w + soft, angle);
    let across = clamp(angle / sun.w, 0.0, 1.0);
    let limb = 1.0 - 0.6 * (1.0 - sqrt(1.0 - across * across));
    return sun_radiance.rgb * (disc * limb * smoothstep(-0.002, 0.004, ray.y));
}

@fragment
fn fragment(mesh: VertexOutput) -> SkyOutput {
    let uv = coords_to_viewport_uv(mesh.position.xy, view.viewport);
    let clouds = textureSampleLevel(clouds_render_texture, clouds_render_sampler, uv, 0.0);
    let sky = textureSampleLevel(sky_texture, clouds_render_sampler, uv, 0.0);
    let far = view.world_from_clip * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0);
    let ray = normalize(far.xyz / far.w - view.world_position);
    // Clouds hide the disc by what they let through, as they do the sky.
    let behind = sky.rgb + sun_disc(ray);
    return SkyOutput(vec4<f32>(max(clouds.rgb + behind * clouds.a, vec3<f32>(0.0)), 1.0), 0.0);
}
