// Animated selection-ring shader — ported from the astrocraft game.
// Renders the annulus as a set of rotating segmented dashes with a
// fine-striped detail pattern inside each dash; pixels outside the
// dashes are `discard`ed so the mesh reads as a spinning necklace
// of light rather than a solid ring.

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

struct SelectionRingSettings {
    color_r: f32,
    color_g: f32,
    color_b: f32,
    time: f32,
    pulse_speed: f32,
    pulse_count: f32,
    alpha: f32,
    center_x: f32,
    center_z: f32,
    // Fine stripes *per coarse segment*. Together with `pulse_count`
    // it determines the total fine-line count around the ring
    // (`pulse_count × fine_mult`). Computed per-vehicle by the
    // Rust side so every ring shows the same visual density
    // regardless of its diameter.
    fine_mult: f32,
    _pad2: f32,
    _pad3: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> settings: SelectionRingSettings;

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

    // Use the annulus mesh's UV-v as the angle around the ring.
    // Bevy's annulus builder sets `uv.y` to go 0 → 1 around the
    // circumference, so this is intrinsically ring-local: it
    // doesn't drift when the mesh moves around the world.
    // (A previous `atan2(world_position − center)` formulation
    // caused the pattern to "crawl" during translation.)
    let norm_angle = in.uv.y;

    // Spinning on/off segments.
    let phase = norm_angle * settings.pulse_count * 6.283185 - settings.time * settings.pulse_speed;
    let pulse = step(0.0, sin(phase));
    if pulse < 0.5 {
        discard;
    }

    // Fine stripes LOCKED to the coarse phase. Setting the fine
    // phase = `fine_mult * phase` guarantees the two patterns share
    // the same zero-crossings: every coarse "on" segment starts
    // exactly at a fine-stripe boundary, and they stay aligned as
    // both rotate in time. The previous `pulse_speed * 3.0` made the
    // fine pattern drift at 3× the coarse rate, which is what made
    // fine stripes appear to slide in and out of the big gaps.
    let fine_phase = phase * settings.fine_mult;
    let fine_pulse = step(0.3, sin(fine_phase));
    if fine_pulse < 0.5 {
        discard;
    }

    // Straight pass-through of the accent colour — no multiplier.
    // Any > 1.0 multiplier clamps bright channels against sRGB and
    // shifts the hue (e.g. bright orange drifts toward yellow-white).
    let base_color = vec3<f32>(settings.color_r, settings.color_g, settings.color_b);
    out.color = vec4<f32>(base_color, settings.alpha);
#endif

    return out;
}
