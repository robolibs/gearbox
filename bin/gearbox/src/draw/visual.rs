use gearbox_api::draw_style;

pub(crate) const INK: [u8; 4] = [224, 239, 247, 255];
pub(crate) const ACCENT: [u8; 4] = [108, 184, 216, 255];
pub(crate) const BACKGROUND: [u8; 4] = [10, 22, 32, 0];
pub(crate) const SECONDARY: [u8; 4] = [194, 205, 210, 255];
pub(crate) const AXIS_COLORS: [[u8; 4]; 3] = [
    [0xD5, 0x5E, 0x00, 255],
    [0x00, 0x9E, 0x73, 255],
    [0x00, 0x72, 0xB2, 255],
];
pub(crate) const RULE_PX: f32 = gearbox_api::wire::draw_widgets::STRUCTURE_RULE_PX;
pub(crate) const CORNER_PX: f32 = 6.0;
pub(crate) const SPACE_PX: f32 = 4.0;
pub(crate) const STRUCTURE_LIMIT: usize = 64;
pub(crate) const STRUCTURE_CORNER_GAIN: f32 =
    gearbox_api::wire::draw_widgets::STRUCTURE_CORNER_GAIN;
pub(crate) const BRACKET_FRACTION: f32 = gearbox_api::wire::draw_widgets::BRACKET_FRACTION;
pub(crate) const FEATURE_EDGE_LIMIT: usize = 16_384;
pub(crate) const ENERGY_HZ: f32 = 0.5;
pub(crate) const FLOW_SPACING_M: f32 = 1.2;
pub(crate) const HALO_MAX_PX: f32 = 4.0;
pub(crate) const HALO_BASE_PX: f32 = 3.6;
pub(crate) const HALO_RADIUS_GAIN: f32 = 0.08;
pub(crate) const HALO_RADIUS_CAP: f32 = 10.0;
pub(crate) const HALO_GAIN: f32 = 0.42;

pub(crate) fn blur(entity: &gearbox_api::DrawEntity, default: f32) -> f32 {
    entity
        .prop_f32("blur")
        .filter(|v| v.is_finite())
        .unwrap_or(default)
        .clamp(0.0, 1.0)
}

pub(crate) fn blur_key(entity: &gearbox_api::DrawEntity) -> u8 {
    (blur(entity, 0.0) * 255.0).round() as u8
}

pub(crate) fn label_opacity(depth: f32, max_distance: Option<f32>) -> f32 {
    if !depth.is_finite() || depth <= 0.0 {
        return 0.0;
    }
    let end = max_distance
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(90.0);
    let start = end * (60.0 / 90.0);
    1.0 - ((depth - start) / (end - start)).clamp(0.0, 1.0)
}

pub(crate) fn halo_radius_px(radius: f32, glow: f32) -> f32 {
    if glow <= 0.0 {
        return 0.0;
    }
    ((HALO_BASE_PX + HALO_RADIUS_GAIN * radius.clamp(0.0, HALO_RADIUS_CAP)) * glow.sqrt())
        .min(HALO_MAX_PX)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EffectDefaults {
    pub glow: f32,
    pub pulse: f32,
    pub flow: f32,
    pub spacing: f32,
    pub beam: f32,
    pub fade: f32,
}

const fn fx(
    glow: f32,
    pulse: f32,
    flow: f32,
    spacing: f32,
    beam: f32,
    fade: f32,
) -> EffectDefaults {
    EffectDefaults {
        glow,
        pulse,
        flow,
        spacing,
        beam,
        fade,
    }
}

const EFFECTS: [EffectDefaults; 21] = [
    fx(0.0, 0.0, 0.0, 0.4, 0.0, 1.0), // auto
    fx(0.0, 0.0, 0.0, 0.4, 0.0, 1.0), // plain
    fx(
        0.70,
        0.0,
        ENERGY_HZ * FLOW_SPACING_M,
        FLOW_SPACING_M,
        0.0,
        1.0,
    ), // ribbon_flow
    fx(0.70, ENERGY_HZ, 0.0, 0.4, 0.0, 0.15), // tube_pulse
    fx(0.0, 0.0, 0.0, 0.4, 0.0, 1.0), // hairline
    fx(0.75, ENERGY_HZ, 0.0, 0.8, 0.25, 0.0), // light_wall
    fx(0.95, ENERGY_HZ, 0.0, 0.4, 3.0, 1.0), // beacon
    fx(0.65, ENERGY_HZ, 0.0, 0.4, 0.8, 1.0), // holo
    fx(0.60, ENERGY_HZ, 0.0, 0.4, 0.0, 1.0), // shield
    fx(0.65, ENERGY_HZ, 0.0, 0.4, 0.0, 1.0), // alert
    fx(0.75, ENERGY_HZ, 0.0, 0.4, 0.0, 1.0), // orb
    fx(0.55, 0.0, 0.0, 0.4, 0.0, 1.0), // decal
    fx(0.70, ENERGY_HZ, 0.0, 0.4, 0.0, 1.0), // arrow_glow
    fx(0.55, ENERGY_HZ, 0.0, 0.4, 0.0, 1.0), // point_glow
    fx(0.0, 0.0, 0.0, 0.4, 0.0, 1.0), // label
    fx(0.0, 0.0, 0.0, 0.0, 0.0, 1.0), // heat
    fx(0.0, 0.0, 0.0, 0.4, 0.0, 1.0), // splat
    fx(0.60, ENERGY_HZ, 0.0, 0.0, 0.0, 1.0), // sweep
    fx(0.60, ENERGY_HZ, 0.0, 0.4, 0.0, 0.15), // ghost
    fx(0.55, 0.0, 0.0, 0.0, 0.0, 1.0), // ruler
    fx(0.60, ENERGY_HZ, 0.0, 1.0, 0.0, 1.0), // field
];

pub(crate) fn defaults(style: u32) -> EffectDefaults {
    EFFECTS
        .get(style as usize)
        .copied()
        .unwrap_or(EFFECTS[draw_style::PLAIN as usize])
}

pub(crate) fn effect(given: f32, default: f32) -> f32 {
    if given > 0.0 {
        given
    } else if given < 0.0 {
        0.0
    } else {
        default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_and_draw_labels_share_distance_fading() {
        for (depth, opacity) in [
            (1.0, 1.0),
            (60.0, 1.0),
            (75.0, 0.5),
            (90.0, 0.0),
            (400.0, 0.0),
        ] {
            assert_eq!(label_opacity(depth, None), opacity);
            assert_eq!(label_opacity(depth * 2.0, Some(180.0)), opacity);
        }
        for depth in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(label_opacity(depth, None), 0.0);
        }
        assert_eq!(label_opacity(75.0, Some(f32::NAN)), 0.5);
    }

    #[test]
    fn every_style_has_a_finite_profile() {
        for style in 0..=draw_style::FIELD {
            assert_eq!(draw_style::parse(draw_style::name(style)), Some(style));
            let e = defaults(style);
            assert!(
                [e.glow, e.pulse, e.flow, e.spacing, e.beam, e.fade]
                    .into_iter()
                    .all(|v| v.is_finite() && v >= 0.0)
            );
            assert!(e.glow <= 1.2 && e.fade <= 1.0);
        }
        assert_eq!(defaults(u32::MAX), defaults(draw_style::PLAIN));
    }

    #[test]
    fn blur_strength_preserves_defaults_and_bounds_material_variants() {
        let entity = gearbox_api::DrawEntity::text("glass", "x", &[[0.0; 3]], "sharp text");
        assert_eq!(blur(&entity, 0.0), 0.0);
        assert_eq!(blur(&entity, 1.0), 1.0);
        for step in 0..=1000 {
            let value = step as f32 / 1000.0;
            let given = entity.clone().with_blur(value);
            assert!((blur(&given, 1.0) - value).abs() < 1e-6);
            assert!((blur_key(&given) as f32 / 255.0 - value).abs() <= 0.5 / 255.0 + 1e-6);
        }
        assert_eq!(blur(&entity.with_blur(0.0), 1.0), 0.0);
    }

    #[test]
    fn explicit_controls_override_every_profile() {
        for style in 0..=draw_style::FIELD {
            let e = defaults(style);
            for default in [e.glow, e.pulse, e.flow, e.spacing, e.beam, e.fade] {
                assert_eq!(effect(-1.0, default), 0.0);
                assert_eq!(effect(0.0, default), default);
                assert_eq!(effect(2.5, default), 2.5);
            }
        }
    }

    #[test]
    fn scientific_and_ordinary_profiles_are_static() {
        for style in [
            draw_style::PLAIN,
            draw_style::HAIRLINE,
            draw_style::SPLAT,
            draw_style::HEAT,
            draw_style::LABEL,
            draw_style::RULER,
            draw_style::DECAL,
        ] {
            assert_eq!((defaults(style).pulse, defaults(style).flow), (0.0, 0.0));
        }
        for style in [
            draw_style::PLAIN,
            draw_style::HAIRLINE,
            draw_style::SPLAT,
            draw_style::HEAT,
            draw_style::LABEL,
        ] {
            assert_eq!(defaults(style).glow, 0.0);
        }
    }

    #[test]
    fn energized_profiles_share_cadence_and_visible_bounded_aura() {
        for style in [
            draw_style::TUBE_PULSE,
            draw_style::LIGHT_WALL,
            draw_style::BEACON,
            draw_style::HOLO,
            draw_style::SHIELD,
            draw_style::ALERT,
            draw_style::ORB,
            draw_style::ARROW_GLOW,
            draw_style::POINT_GLOW,
            draw_style::SWEEP,
            draw_style::GHOST,
            draw_style::FIELD,
        ] {
            let e = defaults(style);
            assert_eq!(e.pulse, ENERGY_HZ);
            assert_eq!(e.flow, 0.0);
            assert!((0.55..=0.95).contains(&e.glow));
            for radius in [0.75, 2.0, 8.0, 40.0] {
                assert!((2.0..=4.0).contains(&halo_radius_px(radius, e.glow)));
            }
        }
        let ribbon = defaults(draw_style::RIBBON_FLOW);
        assert_eq!(ribbon.flow / ribbon.spacing, ENERGY_HZ);
    }

    #[test]
    fn halo_support_is_visible_at_defaults_and_vanishes_when_disabled() {
        for radius in [0.0, 0.75, 4.0, 1000.0] {
            assert_eq!(halo_radius_px(radius, 0.0), 0.0);
            assert_eq!(halo_radius_px(radius, -1.0), 0.0);
            assert!(halo_radius_px(radius, 100.0) <= 4.0);
            let reach = halo_radius_px(radius, 0.55);
            let one_pixel = (-3.0 * (1.0 / reach).powi(2)).exp();
            assert!(HALO_GAIN * 0.55 * one_pixel > 0.1);
        }
    }

    #[test]
    fn cpu_and_shader_halo_tokens_agree() {
        let shader = include_str!("draw_common.wgsl");
        for (name, value) in [
            ("RULE_PX", RULE_PX),
            ("CORNER_PX", CORNER_PX),
            ("HALO_MAX_PX", HALO_MAX_PX),
            ("HALO_BASE_PX", HALO_BASE_PX),
            ("HALO_RADIUS_GAIN", HALO_RADIUS_GAIN),
            ("HALO_RADIUS_CAP", HALO_RADIUS_CAP),
            ("HALO_GAIN", HALO_GAIN),
            ("STRUCTURE_CORNER_GAIN", STRUCTURE_CORNER_GAIN),
            ("BRACKET_FRACTION", BRACKET_FRACTION),
        ] {
            let prefix = format!("const {name}: f32 = ");
            let token = shader
                .lines()
                .find_map(|line| line.strip_prefix(prefix.as_str()))
                .unwrap();
            assert_eq!(token.trim_end_matches(';').parse::<f32>().unwrap(), value);
        }
    }
}
