// Heading / inertia arrows shader — draws a disc of `>>>` chevrons
// flowing outward from the vehicle centre in the direction of motion.
// Chevron count grows with speed; a stationary vehicle produces no
// output at all (fully discarded).
//
// Curvature handling: when the vehicle is turning we *don't* deform
// the chevrons. Each chevron stays a rigid `>` shape — we just place
// it at the correct arc-length position along the curved path and
// rotate it to match the local tangent. The cost is that every
// fragment has to do a point-to-arc projection, but that's one atan2
// + a rotation and the result reads much cleaner than a warped V.

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
    time: f32,
    // XZ unit-vector of the vehicle's horizontal velocity.
    dir_x: f32,
    dir_z: f32,
    // Horizontal speed in m/s (drives chevron scroll rate).
    speed: f32,
    // 0..1 fade factor: 0 when stationary, 1 at `full_intensity_speed`.
    // Controls alpha AND how far out the chevron train extends.
    speed_fade: f32,
    // Outer disc radius in world units.
    radius: f32,
    // Inner cutoff — the selection-ring outer radius + a small margin.
    // Fragments inside this are discarded so chevrons start just
    // outside the halo and never overlap the vehicle body.
    inner_radius: f32,
    // Vehicle world-XZ position, so `world_pos - centre` gives
    // disc-local coords for the chevron math.
    center_x: f32,
    center_z: f32,
    // Signed path curvature (1/m). Positive = turning right
    // (into +v in the velocity frame, which is the vehicle's right),
    // negative = turning left. Zero = straight. Driven by the
    // vehicle's yaw rate / horizontal speed on the Rust side.
    curvature: f32,
    _pad0: f32,
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

    // Nothing to draw when stationary.
    if settings.speed_fade < 0.001 {
        discard;
    }

    // Fragment position relative to the vehicle centre, in world XZ.
    let rel_x = in.world_position.x - settings.center_x;
    let rel_z = in.world_position.z - settings.center_z;

    let r_world = sqrt(rel_x * rel_x + rel_z * rel_z);
    // Keep chevrons strictly outside the selection-ring halo and
    // inside the configured outer disc.
    if r_world < settings.inner_radius || r_world > settings.radius {
        discard;
    }

    // Project onto the velocity frame: u along direction of motion,
    // v perpendicular (+v = vehicle's right in our convention).
    let dir_x = settings.dir_x;
    let dir_z = settings.dir_z;
    let u =  rel_x * dir_x + rel_z * dir_z;
    let v = -rel_x * dir_z + rel_z * dir_x;

    // ── Point-to-arc projection ────────────────────────────────────
    //
    // The vehicle's path is a circular arc passing through the origin
    // tangent to +u, with signed curvature `k`. For `k > 0` the turn
    // centre sits at (0, 1/k) — on the +v side (right). We compute
    // the fragment's arc-length coordinate `s_frag` and skip the
    // straight-line special case when `|k|` is tiny.
    let k = settings.curvature;
    let abs_k = abs(k);

    var s_frag: f32;
    if abs_k < 0.001 {
        s_frag = u;
    } else {
        let inv_k = 1.0 / k;
        // Signed atan2 so arc length grows in +u motion regardless of
        // the sign of k. Reference direction = centre-to-origin.
        let sk = sign(inv_k);
        s_frag = atan2(sk * u, sk * (inv_k - v)) / k;
    }

    // Only chevrons ahead of the vehicle.
    if s_frag < 0.0 {
        discard;
    }

    let spacing = 0.5;
    let line_width = 0.28;
    let line_thickness = line_width * spacing;
    let slope = 1.0;
    let chevron_half_width = 0.38;

    let scroll = settings.time * (settings.speed * 0.6 + 0.8);

    // The chevron this fragment sits inside must have its apex at or
    // *ahead* of `s_frag` so the arms (which trail back in -u_local)
    // can reach it. `ceil` picks the forward-nearest apex.
    let k_idx = ceil((s_frag - scroll) / spacing);
    let apex_s = scroll + k_idx * spacing;

    // ── Chevron apex in world coords + tangent angle ───────────────
    //
    // Rigid-body placement: each chevron is a `>` glued to the arc at
    // `apex_s`. Its forward axis is the arc tangent there. That
    // tangent direction, rotated back into world XZ, becomes the
    // local u_local axis; v_local is the perpendicular.
    var apex_u: f32;       // apex position along +u (world frame)
    var apex_v: f32;       // apex position along +v (world frame)
    var tangent_angle: f32;
    if abs_k < 0.001 {
        apex_u = apex_s;
        apex_v = 0.0;
        tangent_angle = 0.0;
    } else {
        let R = 1.0 / abs_k;
        let phi = apex_s / R;
        apex_u = R * sin(phi);
        // `sign(k)` picks which side of the velocity axis the arc
        // curves toward: k>0 (right turn) → +v, k<0 (left) → -v.
        apex_v = sign(k) * R * (1.0 - cos(phi));
        tangent_angle = apex_s * k;
    }

    // Fragment offset from apex in the velocity frame.
    let du = u - apex_u;
    let dv = v - apex_v;
    // Rotate by -tangent_angle to bring chevron's forward axis onto
    // the local +u_local axis. Chevron is a rigid `>` here.
    let ct = cos(tangent_angle);
    let st = sin(tangent_angle);
    let u_local =  du * ct + dv * st;
    let v_local = -du * st + dv * ct;

    // Rigid chevron body. Apex at origin, arms go into -u_local with
    // slope 1 in |v_local|. The thin line `u_local + |v_local| ≈ 0`
    // is lit; thickness `line_thickness` gives the band.
    if abs(v_local) > chevron_half_width {
        discard;
    }
    let shifted = u_local + abs(v_local) * slope;
    if shifted > 0.0 || shifted < -line_thickness {
        discard;
    }

    // ── Inner / outer clip on the chevron as a whole ───────────────
    //
    // Apex-based so chevrons vanish as whole `>` shapes at the outer
    // edge instead of leaving their arms behind as reverse-V stubs.
    if apex_s < settings.inner_radius {
        discard;
    }
    let max_s = mix(settings.inner_radius, settings.radius, settings.speed_fade);
    let death_fade = 1.0 - smoothstep(max_s - 0.4, max_s, apex_s);
    if death_fade <= 0.001 {
        discard;
    }

    // Soft lateral taper on the arm tips.
    let side_fade = 1.0 - smoothstep(chevron_half_width - 0.1, chevron_half_width, abs(v_local));

    let base_color = vec3<f32>(settings.color_r, settings.color_g, settings.color_b);
    let alpha = 0.85 * settings.speed_fade * death_fade * side_fade;
    out.color = vec4<f32>(base_color, alpha);
#endif

    return out;
}
