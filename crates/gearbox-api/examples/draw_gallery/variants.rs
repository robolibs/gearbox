use gearbox_api::{
    DrawEntity, GridCells, Instance, colormap, draw_flag as flag, draw_style as style, point_style,
    shape_kind as shape,
    wire::{
        draw::color,
        draw_widgets::{self as w, Place},
    },
};

fn label(ns: &str, id: &str, at: [f32; 3], text: &str) -> DrawEntity {
    Place::default()
        .put(DrawEntity::text(ns, &format!("{id}-label"), &[at], text))
        .with_width(-11.0)
        .with_color(color::WHITE)
}

fn line(ns: &str, id: &str, x: f32, y: f32, look: u32) -> DrawEntity {
    let points: Vec<_> = (0..17)
        .map(|i| {
            let t = i as f32 / 16.0;
            [
                x + t * 3.5,
                y + 0.14 * (t * std::f32::consts::TAU).sin(),
                0.12,
            ]
        })
        .collect();
    Place::default()
        .put(DrawEntity::lines(ns, id, &points))
        .with_style(look)
        .with_color(color::CYAN)
        .with_width(if look == style::RIBBON_FLOW {
            0.42
        } else {
            0.07
        })
}

pub fn styles(machine: &str) -> Vec<DrawEntity> {
    let ns = "vfx-gallery/styles";
    let place = Place::default();
    let mut out = Vec::with_capacity(40);
    for look in style::PLAIN..=style::FIELD {
        let i = look - 1;
        let (x, y) = ((i % 5) as f32 * 3.4 - 6.8, 5.5 - (i / 5) as f32 * 3.4);
        let at = [x, y, 0.2];
        let id = style::name(look);
        out.push(label(ns, id, [x, y - 1.2, 0.2], id).with_width(-16.0));
        let e = match look {
            style::BEACON | style::ORB | style::POINT_GLOW | style::SPLAT =>
                DrawEntity::points(ns, id, &[at]).with_width(0.6).with_beam(0.8),
            style::HOLO | style::FIELD | style::SHIELD =>
                DrawEntity::shapes(ns, id, if look == style::SHIELD { shape::ELLIPSOID } else { shape::BOX },
                    &[Instance::at([x, y, 0.7], [1.5, 1.2, 1.1])]).with_spacing(0.5),
            style::DECAL => DrawEntity::polygon(ns, id, &[vec![[x - 0.8, y - 0.6, 0.1], [x + 0.8, y - 0.6, 0.1],
                [x + 0.8, y + 0.6, 0.1], [x - 0.8, y + 0.6, 0.1]]]),
            style::ARROW_GLOW => DrawEntity::arrows(ns, id, &[([x - 0.8, y, 0.2], [x + 0.8, y, 0.2])]).with_width(0.1),
            style::LABEL => DrawEntity::text(ns, id, &[at], "Supplied label").with_width(-16.0)
                .with_prop("label_category", "ANNOTATION").with_prop("label_icon", "measurement"),
            style::HEAT => DrawEntity::grid(ns, id, 3, 3, 0.5, GridCells::U8(vec![0, 20, 40, 30, 255, 60, 60, 80, 100]))
                .at_yaw([x as f64 - 0.75, y as f64 - 0.75, 0.1], 0.0).with_colormap(colormap::VIRIDIS, 0.0, 100.0)
                .with_prop("unknown", "255"),
            style::SWEEP => DrawEntity::shapes(ns, id, shape::SECTOR, &[Instance::at(at, [1.1, 1.6, 0.0])]),
            style::GHOST => DrawEntity::ghost(ns, id, machine, &[Instance::at([x, y, 0.0], [1.0; 3])]),
            _ => DrawEntity::lines(ns, id, &[[x - 0.9, y - 0.15, 0.15], [x, y + 0.15, 0.15], [x + 0.9, y - 0.15, 0.15]])
                .with_width(if look == style::HAIRLINE { -1.2 } else { 0.08 }).with_spacing(0.25).with_beam(0.7),
        };
        out.push(place.put(e).with_style(look).with_color(color::PLAN));
    }
    out
}

pub fn controls() -> Vec<DrawEntity> {
    let ns = "vfx-gallery/controls";
    let place = Place::default();
    let mut out = vec![label(
        ns,
        "title",
        [-7.0, 7.4, 1.1],
        "CONTROLS / 0 = defaults; negative = off; synthetic fixtures",
    )];
    let columns = [-7.0, -2.3, 2.4];
    for (i, (id, text, glow, beam)) in [
        ("wall-default", "wall: default glow + pulse", 0.0, 0.7),
        ("wall-no-glow", "wall: glow < 0; trace remains", -1.0, 0.7),
        ("wall-no-beam", "wall: beam < 0; no curtain", 0.0, -1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let x = columns[i];
        out.push(label(ns, id, [x, 6.5, 1.0], text));
        out.push(
            line(ns, id, x, 5.8, style::LIGHT_WALL)
                .with_glow(glow)
                .with_beam(beam),
        );
    }
    for (i, (id, text, look, hz)) in [
        (
            "tube-default",
            "tube: pulse = 0 (default)",
            style::TUBE_PULSE,
            0.0,
        ),
        (
            "tube-no-pulse",
            "tube: pulse < 0; no motion",
            style::TUBE_PULSE,
            -1.0,
        ),
        (
            "alert-no-pulse",
            "alert: pulse < 0; static",
            style::ALERT,
            -1.0,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let x = columns[i];
        out.push(label(ns, id, [x, 4.4, 0.5], text));
        out.push(line(ns, id, x, 3.8, look).with_pulse(hz));
    }
    for (i, (id, text, flow, glow)) in [
        ("ribbon-default", "ribbon: flow = 0 (default)", 0.0, 0.0),
        ("ribbon-no-flow", "ribbon: flow < 0; static", -1.0, 0.0),
        ("ribbon-no-glow", "ribbon: glow < 0; body only", 0.0, -1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let x = columns[i];
        out.push(label(ns, id, [x, 2.4, 0.5], text));
        out.push(
            line(ns, id, x, 1.8, style::RIBBON_FLOW)
                .with_flow(flow, 0.65)
                .with_glow(glow),
        );
    }
    for (i, (id, text, glow, flags)) in [
        ("orb-default", "orb: default", 0.0, 0),
        ("orb-no-glow", "orb: glow < 0", -1.0, 0),
        (
            "orb-add-no-glow",
            "orb: ADD + glow < 0",
            -1.0,
            flag::ADDITIVE,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let x = columns[i];
        out.push(label(ns, id, [x, 0.45, 0.5], text));
        out.push(
            place
                .put(DrawEntity::points(ns, id, &[[x + 1.75, -0.2, 0.45]]))
                .with_style(style::ORB)
                .with_width(0.65)
                .with_color(color::ORANGE)
                .with_glow(glow)
                .with_flags(flags),
        );
    }
    for (i, (id, text, look)) in [
        (
            "alpha-line",
            "alpha: visible / zero below",
            style::RIBBON_FLOW,
        ),
        (
            "alpha-point",
            "point: visible / zero right",
            style::POINT_GLOW,
        ),
        ("alpha-holo", "holo: visible / zero right", style::HOLO),
    ]
    .into_iter()
    .enumerate()
    {
        let x = columns[i];
        out.push(label(ns, id, [x, -1.45, 0.6], text));
        for (hidden, alpha) in [(false, 1.0), (true, 0.0)] {
            let key = format!("{id}-{}", if hidden { "hidden" } else { "visible" });
            let tint = color::with_alpha(color::MAGENTA, alpha);
            let e = match look {
                style::RIBBON_FLOW => {
                    line(ns, &key, x, -2.0 - if hidden { 0.5 } else { 0.0 }, look)
                }
                style::POINT_GLOW => place
                    .put(DrawEntity::points(
                        ns,
                        &key,
                        &[[x + if hidden { 2.7 } else { 0.8 }, -2.2, 0.45]],
                    ))
                    .with_style(look)
                    .with_width(0.65),
                _ => place
                    .put(DrawEntity::shapes(
                        ns,
                        &key,
                        shape::BOX,
                        &[Instance::at(
                            [x + if hidden { 2.7 } else { 0.8 }, -2.2, 0.45],
                            [0.65; 3],
                        )],
                    ))
                    .with_style(look)
                    .with_fill(flag::FILL_SOLID)
                    .with_beam(-1.0),
            };
            out.push(e.with_color(tint));
        }
    }
    for (i, (id, text, width)) in [
        ("metre-width", "plain: 0.08 metres", 0.08),
        ("pixel-width", "plain: 4 pixels", -4.0),
    ]
    .into_iter()
    .enumerate()
    {
        let x = columns[i];
        out.push(label(ns, id, [x, -3.6, 0.5], text));
        out.push(line(ns, id, x, -4.2, style::PLAIN).with_width(width));
    }
    out.push(label(
        ns,
        "point-shapes",
        [columns[2], -3.6, 0.5],
        "PLAIN: sphere / disc (not SPLAT)",
    ));
    for (id, x, sub) in [
        ("actual-sphere", 3.2, point_style::SPHERE),
        ("actual-disc", 5.1, point_style::DISC),
    ] {
        out.push(
            place
                .put(DrawEntity::points(ns, id, &[[x, -4.2, 0.45]]))
                .with_style(style::PLAIN)
                .with_sub(sub)
                .with_width(0.75)
                .with_color(color::GREEN),
        );
    }
    let points = [
        [-7.0, -6.2, 0.12],
        [-4.8, -6.0, 0.12],
        [-2.4, -6.2, 0.12],
        [0.0, -6.0, 0.12],
    ];
    out.push(label(
        ns,
        "wall-gradient",
        [-7.0, -5.5, 1.1],
        "wall: per-point RGB + alpha; pulse < 0",
    ));
    out.push(
        place
            .put(DrawEntity::lines(ns, "wall-gradient", &points))
            .with_style(style::LIGHT_WALL)
            .with_width(-3.0)
            .with_beam(0.8)
            .with_pulse(-1.0)
            .with_colors(&[
                color::CYAN,
                color::GREEN,
                color::ORANGE,
                color::with_alpha(color::MAGENTA, 0.25),
            ]),
    );
    out.push(label(
        ns,
        "holo-no-pulse",
        [2.4, -5.5, 1.0],
        "holo: pulse < 0; beam static",
    ));
    out.push(
        place
            .put(DrawEntity::shapes(
                ns,
                "holo-no-pulse",
                shape::BOX,
                &[Instance::at([4.1, -6.2, 0.4], [0.65; 3])],
            ))
            .with_style(style::HOLO)
            .with_color(color::CYAN)
            .with_beam(0.65)
            .with_pulse(-1.0),
    );
    out
}

pub fn dense() -> Vec<DrawEntity> {
    let ns = "vfx-gallery/dense";
    let place = Place::default();
    let mut out = vec![
        label(
            ns,
            "cloud-title",
            [-7.4, 6.7, 1.4],
            "SYNTHETIC / 20,000 static scalar splats; VIRIDIS",
        ),
        label(
            ns,
            "rollout-title",
            [0.4, 6.7, 1.4],
            "SYNTHETIC / 480 static cost rollouts; magenta = collision",
        ),
    ];
    let mut points = Vec::with_capacity(20_000);
    let mut values = Vec::with_capacity(20_000);
    for iy in 0..200 {
        for ix in 0..100 {
            let u = ix as f32 / 99.0;
            let v = iy as f32 / 199.0;
            let scalar = (0.5 + 0.3 * (u * 6.0).sin() * (v * 8.0).cos() + 0.2 * v).clamp(0.0, 1.0);
            points.push([-7.4 + 6.5 * u, -5.7 + 11.4 * v, 0.3 + 0.8 * scalar]);
            values.push(scalar);
        }
    }
    let cloud = place
        .put(DrawEntity::points(ns, "scalar-cloud-20000", &points))
        .with_style(style::SPLAT)
        .with_width(-2.2)
        .with_values(&values)
        .with_colormap(colormap::VIRIDIS, 0.0, 1.0);
    out.push(cloud);
    let mut strips = Vec::with_capacity(480);
    let mut costs = Vec::with_capacity(480);
    let mut collisions = Vec::with_capacity(480);
    for k in 0..480 {
        let spread = 2.0 * k as f32 / 479.0 - 1.0;
        strips.push(
            (0..64)
                .map(|i| {
                    let t = i as f32 / 63.0;
                    [
                        0.5 + 6.9 * t,
                        spread * (0.5 + 4.6 * t * t)
                            + 0.16 * (t * 9.0).sin() * (k as f32 * 0.37).sin(),
                        0.25,
                    ]
                })
                .collect(),
        );
        costs.push(spread.abs());
        collisions.push(k % 47 == 0);
    }
    for e in w::rollouts(
        &place,
        ns,
        "cost-rollouts-480",
        &strips,
        &costs,
        &collisions,
        1,
    ) {
        out.push(e.with_width(-0.8));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_has_complete_logical_cloud_and_rollouts() {
        let entities = dense();
        let cloud = entities
            .iter()
            .find(|e| e.id() == "scalar-cloud-20000")
            .unwrap();
        assert_eq!(cloud.point_count(), 20_000);
        let rollouts: Vec<_> = entities
            .iter()
            .filter(|e| e.ns().ends_with("/cost-rollouts-480"))
            .collect();
        assert_eq!(
            rollouts
                .iter()
                .map(|e| e.strip_ranges().len())
                .sum::<usize>(),
            480
        );
        assert_eq!(
            rollouts.iter().map(|e| e.point_count()).sum::<usize>(),
            480 * 64
        );
        for e in entities {
            assert_eq!(e.op, gearbox_api::draw_op::UPSERT);
            e.validate().unwrap();
        }
    }

    #[test]
    fn controls_are_valid_and_keep_zero_alpha_explicit() {
        let entities = controls();
        for e in &entities {
            e.validate().unwrap();
            assert!(e.ns().starts_with("vfx-gallery/controls"));
        }
        for id in [
            "alpha-line-hidden",
            "alpha-point-hidden",
            "alpha-holo-hidden",
        ] {
            let e = entities.iter().find(|e| e.id() == id).unwrap();
            assert_ne!(e.color, 0);
            assert_eq!(e.color >> 24, 0);
        }
    }
}
