// Heading indicator shader — one rigid `>` chevron sitting just past
// the halo, aimed along the vehicle's velocity direction. No scroll,
// no bend, no multi-chevron train: alpha is the only thing that
// animates, driven by the smoothed `speed_fade` on the Rust side.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

struct HeadingArrowsSettings {
    color_r: f32,
    color_g: f32,
    color_b: f32,
    // 0..1 intensity, smoothed on the Rust side with asymmetric
    // rise / fall rates so the chevron fades in fast and out slow.
    speed_fade: f32,
    // XZ unit-vector of the vehicle's horizontal velocity (smoothed).
    dir_x: f32,
    dir_z: f32,
    // Distance from vehicle centre to the chevron apex, in metres.
    // Computed on the Rust side as `inner_radius + small offset` so
    // the chevron always sits clear of the halo.
    apex_u: f32,
    _pad0: f32,
    // Vehicle world-XZ position for the world-to-local transform.
    center_x: f32,
    center_z: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> settings: HeadingArrowsSettings;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;

    if settings.speed_fade < 0.001 {
        discard;
    }

    // Fragment position in the velocity frame (u forward, v right).
    let rel_x = in.world_position.x - settings.center_x;
    let rel_z = in.world_position.z - settings.center_z;
    let u =  rel_x * settings.dir_x + rel_z * settings.dir_z;
    let v = -rel_x * settings.dir_z + rel_z * settings.dir_x;

    // Single rigid `>` centred on (apex_u, 0). Arms trail back in
    // `-u_local`; apex points in `+u_local`.
    let u_local = u - settings.apex_u;
    let v_local = v;

    let slope = 1.0;
    let chevron_half_width = 0.55;
    let line_thickness = 0.18;

    if abs(v_local) > chevron_half_width {
        discard;
    }
    let shifted = u_local + abs(v_local) * slope;
    if shifted > 0.0 || shifted < -line_thickness {
        discard;
    }

    // Soft taper on the arm tips so the `>` doesn't clip with a
    // pixel edge at the lateral extents.
    let side_fade = 1.0
        - smoothstep(chevron_half_width - 0.12, chevron_half_width, abs(v_local));

    let base_color = vec3<f32>(settings.color_r, settings.color_g, settings.color_b);
    let alpha = 0.9 * settings.speed_fade * side_fade;
    out.color = vec4<f32>(base_color, alpha);
#endif

    return out;
}
