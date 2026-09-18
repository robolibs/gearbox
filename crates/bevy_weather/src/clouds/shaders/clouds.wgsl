#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
    utils::coords_to_viewport_uv,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var clouds_render_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var clouds_render_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var sky_texture: texture_2d<f32>;

struct SkyOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fragment(mesh: VertexOutput) -> SkyOutput {
    let uv = coords_to_viewport_uv(mesh.position.xy, view.viewport);
    let clouds = textureSampleLevel(clouds_render_texture, clouds_render_sampler, uv, 0.0);
    let sky = textureSampleLevel(sky_texture, clouds_render_sampler, uv, 0.0);
    return SkyOutput(vec4<f32>(max(clouds.rgb + sky.rgb * clouds.a, vec3<f32>(0.0)), 1.0), 0.0);
}
