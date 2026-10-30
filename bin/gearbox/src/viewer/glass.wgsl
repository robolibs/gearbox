// Glass fading out over the last stretch before the eye, on the fragments
// nearest it: a pane the eye flies through opens a clear spot in its haze.
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct NearFade { band: vec4<f32> }
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> near_fade: NearFade;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let away = distance(in.world_position.xyz, view.world_position);
    pbr_input.material.base_color.a *= smoothstep(near_fade.band.x, near_fade.band.y, away);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
