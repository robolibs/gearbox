use super::*;

pub(super) fn manifest() -> serde_json::Value {
    serde_json::json!({"scenes": [
        {"id": "blur-review", "focus": [0.0, 0.0, 1.0], "distance": 23, "pitch": 55, "bearing": 180,
            "bounds": {"min": [-10.0, -11.0, 0.0], "max": [10.0, 11.0, 3.5]}},
        {"id": "review", "focus": [0.0, 0.0, 0.8], "distance": 30, "pitch": 72, "bearing": 180,
            "bounds": {"min": [-13.0, -7.0, 0.0], "max": [13.0, 7.0, 4.0]}},
        {"id": "energy-review", "focus": [0.0, 0.0, 1.0], "distance": 25, "pitch": 50, "bearing": 180,
            "bounds": {"min": [-13.0, -7.5, 0.0], "max": [13.0, 7.0, 4.0]}},
        {"id": "review-colors", "focus": [0.0, 0.0, 0.8], "distance": 30, "pitch": 72, "bearing": 180,
            "bounds": {"min": [-13.0, -7.0, 0.0], "max": [13.0, 7.0, 4.0]}},
        {"id": "review-colors-light", "focus": [0.0, 0.0, 0.8], "distance": 30, "pitch": 72, "bearing": 180,
            "bounds": {"min": [-13.0, -7.0, 0.0], "max": [13.0, 7.0, 4.0]}},
        {"id": "review-finishes", "focus": [0.0, 0.0, 0.8], "distance": 30, "pitch": 72, "bearing": 180,
            "bounds": {"min": [-13.0, -7.0, 0.0], "max": [13.0, 7.0, 4.0]}},
        {"id": "review-system", "focus": [0.0, 0.0, 0.8], "distance": 30, "pitch": 55, "bearing": 180,
            "bounds": {"min": [-13.0, -8.0, 0.0], "max": [13.0, 8.0, 4.0]}},
        {"id": "structure-language", "focus": [0.0, 0.0, 0.8], "distance": 34, "pitch": 55, "bearing": 160,
            "bounds": {"min": [-16.0, -11.0, 0.0], "max": [16.0, 11.0, 4.0]}},
        {"id": "region-language", "focus": [0.0, 0.0, 0.8], "distance": 29, "pitch": 62, "bearing": 160,
            "bounds": {"min": [-11.0, -9.0, 0.0], "max": [11.0, 9.0, 4.0]}},
        {"id": "energy-colors", "focus": [0.0, 0.0, 1.0], "distance": 25, "pitch": 50, "bearing": 180,
            "bounds": {"min": [-13.0, -7.5, 0.0], "max": [13.0, 7.0, 4.0]}}
    ]})
}

pub(super) fn blur_review(machine: &str) -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/blur-review");
    let mut out = Vec::new();
    for (column, blur) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        let x = column as f32 * 6.0 - 6.0;
        let tag = format!("c{column}");
        out.push(p.put(E::text(&ns, &format!("{tag}-title"), &[[x, 10.5, 0.8]], &format!("BLUR / {blur:.1}"))).with_width(-20.0));
        let tint = color::CYAN;
        let mut group = vec![
            p.put(E::lines(&ns, &format!("{tag}-wall"), &[[x - 2.0, 8.0, 0.08], [x, 8.5, 0.08], [x + 2.0, 8.0, 0.08]]))
                .with_style(style::LIGHT_WALL).with_beam(0.45).with_width(0.028).with_color(color::ORANGE).with_fade(-1.0),
            p.put(E::shapes(&ns, &format!("{tag}-sphere"), shape::ELLIPSOID, &[Instance::at([x - 1.15, 3.7, 1.0], [1.8; 3])]))
                .with_style(style::HOLO).with_color(tint).with_beam(-1.0),
            p.put(E::shapes(&ns, &format!("{tag}-cone"), shape::CONE, &[Instance::at([x + 1.2, 3.7, 0.9], [1.7, 1.7, 1.8])]))
                .with_style(style::SHIELD).with_color(color::MAGENTA).with_beam(-1.0),
            p.put(E::polygon(&ns, &format!("{tag}-zone"), &[rect(x - 2.0, -0.5, 4.0, 2.0)]))
                .with_style(style::DECAL).with_fill(flag::FILL_BOTH).with_color(color::GREEN),
            p.put(E::ghost(&ns, &format!("{tag}-robot"), machine, &[Instance::at([x, -4.0, 0.4], [1.0; 3])]))
                .with_color(color::CYAN),
            w::table(&p, &ns, &format!("{tag}-panel"), [x, -8.0, 1.0],
                &[("Blur".into(), format!("{blur:.1}")), ("Data".into(), "Unfiltered".into())], "SHARED GLASS")
                .with_color(color::MAGENTA).with_prop("panel_finish", "white"),
        ];
        for entity in &mut group { *entity = entity.clone().with_blur(blur).with_glow(0.55); }
        out.extend(group);
    }
    out
}

pub(super) fn panels() -> Vec<E> {
    panels_with_colors(false)
}

pub(super) fn panel_colors() -> Vec<E> {
    panels_with_colors(true)
}

pub(super) fn panel_colors_light() -> Vec<E> {
    panel_colors().into_iter().map(|e| e.with_prop("panel_theme", "light")).collect()
}

pub(super) fn panel_finishes() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/review-finishes");
    let samples: Vec<_> = (0..48).map(|i| 0.4 + 0.25 * (i as f32 * 0.22).sin()).collect();
    let mut panels = Vec::with_capacity(6);
    for (i, (finish, title)) in [("color", "COLOR"), ("white", "WHITE + COLOR"), ("black", "BLACK + COLOR")].into_iter().enumerate() {
        let x = (i as f32 - 1.0) * 9.0;
        for (row, tint) in [color::MAGENTA, color::CYAN].into_iter().enumerate() {
            let id = format!("{finish}-{row}");
            let e = if row == 0 {
                w::gauge(&p, &ns, &id, [x, 5.0, 0.8], 0.65, 0.0, 1.0, title, "m")
            } else {
                w::panel(&p, &ns, &id, [x, -5.0, 0.8], panel_kind::SPARKLINE, &samples, &[], title)
                    .with_prop("min", "0").with_prop("max", "1").with_prop("unit", "m")
            };
            panels.push(e.with_color(tint).with_prop("label_accent", &tint.to_string())
                .with_prop("panel_theme", "light").with_prop("panel_finish", finish).with_prop("label_max_distance", "70"));
        }
    }
    panels
}

pub(super) fn system(machine: &str) -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/review-system");
    let mut out = Vec::new();
    for (id, title, x, y, tint) in [
        ("holo", "Hologram", -9.0, 5.0, color::CYAN),
        ("field", "Field volume", 0.0, 5.0, color::MAGENTA),
        ("shield", "Shield", 9.0, 5.0, color::ORANGE),
        ("ghost", "Robot ghost", -9.0, -5.0, color::GREEN),
        ("camera", "Camera frustum", 0.0, -5.0, color::YELLOW),
    ] {
        let entity = match id {
            "holo" => E::shapes(&ns, id, shape::BOX, &[Instance::at([x, y, 1.1], [3.5, 2.4, 2.0])]).with_style(style::HOLO),
            "field" => E::shapes(&ns, id, shape::BOX, &[Instance::at([x, y, 1.1], [3.5, 2.4, 2.0])]).with_style(style::FIELD).with_spacing(0.8),
            "shield" => E::shapes(&ns, id, shape::ELLIPSOID, &[Instance::at([x, y, 1.1], [3.5, 2.4, 2.0])]).with_style(style::SHIELD),
            "ghost" => E::ghost(&ns, id, machine, &[Instance::at([x, y, 0.0], [1.0; 3])]).with_fade(-1.0),
            "camera" => E::shapes(&ns, id, shape::FRUSTUM, &[Instance::at([x - 1.5, y, 1.1], [1.0, 0.7, 3.0])])
                .with_prop("near", "2.0").with_style(style::SWEEP),
            _ => unreachable!(),
        };
        out.push(p.put(entity).with_color(tint));
        out.push(p.put(E::text(&ns, &format!("{id}-label"), &[[x, y - 2.6, 0.15]], title))
            .with_width(-16.0).with_color(color::WHITE).with_prop("label_accent", &tint.to_string())
            .with_prop("label_selected", if id == "ghost" { "true" } else { "false" }).with_prop("label_max_distance", "70"));
    }
    out.push(w::table(&p, &ns, "table", [9.0, -5.0, 0.8], &[
        ("Sample".into(), "027".into()), ("Distance".into(), "0.65 m".into()), ("Source".into(), "Fixture".into()),
    ], "Inspection data").with_color(color::CYAN).with_prop("panel_finish", "white").with_prop("label_max_distance", "70"));
    out
}

pub(super) fn region_language() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/region-language");
    let mut out = Vec::new();
    for (i, (kind, size, look, name)) in [
        (shape::ELLIPSE, [4.0, 3.0, 0.0], style::DECAL, "Ellipse / curved border accents"),
        (shape::RECT, [4.0, 3.0, 0.0], style::DECAL, "Region / fine rule and stronger ends"),
        (shape::SECTOR, [2.0, std::f32::consts::TAU, 0.9], style::DECAL, "Annulus / empty inner boundary"),
        (shape::SECTOR, [2.0, 4.5, 0.9], style::DECAL, "Wide sector / concave fill"),
        (shape::SECTOR, [2.0, std::f32::consts::TAU, 0.9], style::PLAIN, "Tilted annulus / plain reference"),
        (shape::SECTOR, [2.0, std::f32::consts::TAU, 0.9], style::SWEEP, "Sweep / shared extent and energy"),
    ].into_iter().enumerate() {
        let (x, y) = ((i % 3) as f32 * 8.0 - 8.0, 5.0 - (i / 3) as f32 * 10.0);
        let tint = [color::MAGENTA, color::GREEN, color::ORANGE, color::CYAN, color::YELLOW, color::GREEN][i];
        let mut inst = Instance::at([x, y, if i == 4 { 1.8 } else { 0.2 }], size);
        if i == 4 { inst.rotation = [0.4_f32.cos(), 0.0, 0.4_f32.sin(), 0.0]; }
        out.push(p.put(E::shapes(&ns, &format!("region-{i}"), kind, &[inst]))
            .with_style(look).with_fill(flag::FILL_BOTH).with_color(tint).with_width(-1.0));
        out.push(p.put(E::text(&ns, &format!("caption-{i}"), &[[x, y - 2.9, 0.1]], name)).with_width(-15.0));
    }
    out.into_iter().map(|e| e.with_prop("label_max_distance", "90")).collect()
}

pub(super) fn structure_language() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/structure-language");
    let mut out = Vec::new();
    for (i, (kind, size, name)) in [
        (shape::ELLIPSOID, [2.8, 2.8, 2.8], "Sphere / short ring accents"),
        (shape::ELLIPSE, [3.6, 2.6, 0.0], "Ellipse / curved accents"),
        (shape::CYLINDER, [2.5, 2.5, 2.8], "Cylinder / rim and supports"),
        (shape::CONE, [3.0, 3.0, 2.8], "Cone / base and apex"),
        (shape::CAPSULE, [1.8, 1.8, 3.4], "Capsule / continuous curves"),
        (shape::FRUSTUM, [1.0, 0.8, 3.2], "Camera / framed extent"),
        (shape::SECTOR, [2.0, 2.1, 0.7], "Sector / arc and endcaps"),
    ].into_iter().enumerate() {
        let (x, y) = ((i % 4) as f32 * 8.0 - 12.0, 7.0 - (i / 4) as f32 * 7.0);
        let tint = [color::CYAN, color::MAGENTA, color::GREEN, color::ORANGE, color::YELLOW][i % 5];
        out.push(p.put(E::shapes(&ns, &format!("shape-{i}"), kind, &[Instance::at([x, y, 1.6], size)]))
            .with_style(if matches!(kind, shape::FRUSTUM | shape::SECTOR) { style::SWEEP } else { style::HOLO })
            .with_fill(flag::FILL_BOTH).with_color(tint).with_beam(-1.0));
        out.push(p.put(E::text(&ns, &format!("caption-{i}"), &[[x, y - 2.6, 0.2]], name)).with_width(-15.0));
    }
    out.push(p.put(E::polygon(&ns, "region", &[rect(10.2, -1.8, 3.6, 3.6), rect(11.3, -0.7, 1.4, 1.4)]))
        .with_style(style::DECAL).with_fill(flag::FILL_BOTH).with_color(color::ORANGE).with_width(-1.0));
    out.push(p.put(E::text(&ns, "caption-region", &[[12.0, -2.6, 0.2]], "Region / outer and hole boundaries")).with_width(-15.0));
    let asset = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/draw_vfx/faceted.obj");
    out.push(p.put(E::mesh(&ns, "mesh", asset).with_instances(&[Instance::at([-12.0, -7.0, 1.2], [2.0; 3])]))
        .with_style(style::GHOST).with_color(color::MAGENTA));
    out.push(p.put(E::text(&ns, "caption-mesh", &[[-12.0, -9.6, 0.2]], "Mesh / genuine structural edges")).with_width(-15.0));
    out.push(w::gauge(&p, &ns, "gauge", [-4.0, -7.0, 1.2], 0.65, 0.0, 1.0, "WHITE + COLOR", "m")
        .with_color(color::MAGENTA).with_prop("panel_finish", "white"));
    out.push(w::panel(&p, &ns, "chart", [4.0, -7.0, 1.2], panel_kind::SPARKLINE,
        &[0.2, 0.6, 0.4, 0.8, 0.65], &[], "BLACK + COLOR").with_color(color::GREEN).with_prop("panel_finish", "black"));
    out.push(w::table(&p, &ns, "table", [12.0, -7.0, 1.2], &[("Range".into(), "0.65 m".into()), ("Source".into(), "Fixture".into())], "COLOR GLASS")
        .with_color(color::ORANGE).with_prop("panel_finish", "color"));
    out.into_iter().map(|e| e.with_prop("label_max_distance", "90")).collect()
}

fn panels_with_colors(varied: bool) -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/{}", if varied { "review-colors" } else { "review" });
    let samples: Vec<_> = (0..48).map(|i| 0.4 + 0.25 * (i as f32 * 0.22).sin()).collect();
    vec![
        w::panel(&p, &ns, "sparkline", [-9.0, 5.0, 0.8], panel_kind::SPARKLINE, &samples, &[], "Sample history")
            .with_prop("min", "0").with_prop("max", "1").with_prop("unit", "m"),
        w::panel(&p, &ns, "bars", [0.0, 5.0, 0.8], panel_kind::BARS, &[0.72, 0.44, 0.88, 0.31],
            &["A".into(), "B".into(), "C".into(), "D".into()], "Sample channels")
            .with_prop("min", "0").with_prop("max", "1"),
        w::gauge(&p, &ns, "gauge", [9.0, 5.0, 0.8], 0.65, 0.0, 1.0, "Measured range", "m"),
        w::chip(&p, &ns, "chip", [-9.0, -5.0, 0.8], "Fixture annotation", color::PLAN)
            .with_prop("title", "Annotation"),
        w::table(&p, &ns, "table", [0.0, -5.0, 0.8], &[
            ("Sample".into(), "027".into()), ("Distance".into(), "0.65 m".into()), ("Source".into(), "Fixture".into()),
        ], "Inspection data"),
        p.put(w::legend(&ns, "legend", colormap::VIRIDIS, 0.0, 1.0, "Scalar scale", "m"))
            .at_yaw([9.0, -5.0, 0.8], 0.0).with_prop("world_anchor", "true"),
    ].into_iter().enumerate().map(|(i, e)| {
        let tint = if varied { [color::GREEN, color::MAGENTA, color::ORANGE, color::YELLOW, color::CYAN, color::WHITE][i] } else { color::PLAN };
        let e = e.with_color(tint).with_prop("label_accent", &tint.to_string()).with_prop("label_max_distance", "70");
        if varied && e.sub == panel_kind::BARS {
            e.with_colors(&[color::GREEN, color::ORANGE, color::MAGENTA, color::CYAN])
        } else { e }
    }).collect()
}

pub(super) fn energy(machine: &str) -> Vec<E> {
    energy_with_colors(machine, false)
}

pub(super) fn energy_colors(machine: &str) -> Vec<E> {
    energy_with_colors(machine, true)
}

fn energy_with_colors(machine: &str, varied: bool) -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/{}", if varied { "energy-colors" } else { "energy-review" });
    let positions = [-10.5, -3.5, 3.5, 10.5];
    let mut out = Vec::with_capacity(16);
    for (i, (id, caption, look)) in [
        ("wall", "LIGHT WALL", style::LIGHT_WALL), ("ribbon", "FLOW RIBBON", style::RIBBON_FLOW),
        ("tube", "PULSE TUBE", style::TUBE_PULSE), ("beacon", "BEACON", style::BEACON),
        ("holo", "HOLOGRAM", style::HOLO), ("shield", "SHIELD", style::SHIELD),
        ("sweep", "SENSOR SWEEP", style::SWEEP), ("ghost", "ROBOT GHOST", style::GHOST),
    ].into_iter().enumerate() {
        let x = positions[i % 4];
        let y = if i < 4 { 4.5 } else { -4.5 };
        let tint = if varied { [color::GREEN, color::MAGENTA, color::ORANGE, color::YELLOW,
            color::CYAN, color::RED, color::WHITE, color::BLUE][i] } else { color::PLAN };
        let curve: Vec<_> = (0..25).map(|k| {
            let t = k as f32 / 24.0;
            [x - 2.0 + 4.0 * t, y + 0.38 * (t * std::f32::consts::TAU).sin(), 0.15]
        }).collect();
        let e = match look {
            style::LIGHT_WALL => E::lines(&ns, id, &curve).with_width(0.12).with_beam(1.7),
            style::RIBBON_FLOW => E::lines(&ns, id, &curve).with_width(0.55).with_spacing(0.9),
            style::TUBE_PULSE => E::lines(&ns, id, &curve).with_width(0.18),
            style::BEACON => E::points(&ns, id, &[[x, y, 0.15]]).with_width(1.7).with_beam(2.5),
            style::HOLO => E::shapes(&ns, id, shape::BOX, &[Instance::at([x, y, 1.1], [3.5, 2.0, 2.0])]).with_beam(1.8),
            style::SHIELD => E::shapes(&ns, id, shape::ELLIPSOID, &[Instance::at([x, y, 1.2], [3.5, 2.6, 2.2])]),
            style::SWEEP => E::shapes(&ns, id, shape::SECTOR, &[Instance::at([x - 1.5, y, 0.15], [3.2, 1.5, 0.2])]).with_spacing(0.6),
            style::GHOST if varied => E::ghost(&ns, id, machine, &[
                Instance::at([x - 1.7, y, 0.0], [1.0; 3]), Instance::at([x + 1.7, y, 0.0], [1.0; 3]),
            ]).with_colors(&[color::GREEN, color::MAGENTA]).with_fade(-1.0),
            style::GHOST => E::ghost(&ns, id, machine, &[Instance::at([x, y, 0.0], [1.0; 3])]),
            _ => unreachable!(),
        };
        out.push(p.put(e).with_style(look).with_color(tint));
        out.push(p.put(E::text(&ns, &format!("{id}-label"), &[[x, y - 2.6, 0.15]], caption))
            .with_width(-16.0).with_color(color::WHITE).with_prop("label_accent", &tint.to_string())
            .with_prop("label_max_distance", "70"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gearbox_api::draw_kind;
    use std::collections::BTreeSet;

    #[test]
    fn six_panels_use_separate_world_anchors_and_authoritative_values() {
        let items = panels();
        assert_eq!(items.len(), 6);
        let kinds: BTreeSet<_> = items.iter().map(|e| e.sub).collect();
        assert_eq!(kinds, (0..=panel_kind::LEGEND).collect());
        let anchors: BTreeSet<_> = items.iter().map(|e| (e.px as i32, e.py as i32)).collect();
        assert_eq!(anchors, [-9, 0, 9].into_iter().flat_map(|x| [-5, 5].map(|y| (x, y))).collect());
        for e in &items {
            e.validate().unwrap();
            assert_eq!(e.kind, draw_kind::PANEL);
            assert_eq!(e.color, color::PLAN);
            assert_eq!(e.flags & flag::ON_TOP, 0);
        }
        let legend = items.iter().find(|e| e.sub == panel_kind::LEGEND).unwrap();
        assert_eq!(legend.props().get("world_anchor").as_deref(), Some("true"));
        assert_eq!((legend.colormap, legend.vmin, legend.vmax), (colormap::VIRIDIS, 0.0, 1.0));
        let gauge = items.iter().find(|e| e.sub == panel_kind::GAUGE).unwrap();
        assert_eq!(gauge.values, [0.65]);
        assert_eq!(gauge.props().get("unit").as_deref(), Some("m"));
    }

    #[test]
    fn eight_energy_families_keep_shared_defaults_and_consistent_tint() {
        let items = energy("fixture_robot");
        assert_eq!(items.len(), 16);
        let geometry: Vec<_> = items.iter().filter(|e| e.kind != draw_kind::TEXT).collect();
        let looks: BTreeSet<_> = geometry.iter().map(|e| e.style).collect();
        assert_eq!(looks, [style::LIGHT_WALL, style::RIBBON_FLOW, style::TUBE_PULSE, style::BEACON,
            style::HOLO, style::SHIELD, style::SWEEP, style::GHOST].into_iter().collect());
        for e in &geometry {
            e.validate().unwrap();
            assert_eq!(e.color, color::PLAN);
            assert_eq!((e.glow, e.pulse_hz, e.flow_speed), (0.0, 0.0, 0.0));
        }
        assert!(items.iter().all(|e| e.kind != draw_kind::INTERACT && e.kind != draw_kind::PANEL));
    }

    #[test]
    fn caller_colors_change_accents_not_values_or_legend_mapping() {
        let normal = panels();
        let varied = panel_colors();
        for (a, b) in normal.iter().zip(&varied) {
            b.validate().unwrap();
            assert_eq!((a.sub, &a.values, a.colormap, a.vmin, a.vmax), (b.sub, &b.values, b.colormap, b.vmin, b.vmax));
            assert_eq!(b.props().get("label_accent"), Some(b.color.to_string()));
        }
        assert_eq!(varied[1].colors, [color::GREEN, color::ORANGE, color::MAGENTA, color::CYAN]);
        for (dark, light) in varied.iter().zip(panel_colors_light()) {
            light.validate().unwrap();
            assert_eq!(light.props().get("panel_theme").as_deref(), Some("light"));
            assert_eq!((dark.sub, &dark.values, &dark.colors, dark.color, dark.colormap),
                (light.sub, &light.values, &light.colors, light.color, light.colormap));
        }
        let items = energy_colors("fixture_robot");
        assert_eq!(items.chunks_exact(2).map(|pair| pair[0].color).collect::<BTreeSet<_>>().len(), 8);
        for pair in items.chunks_exact(2) {
            assert_eq!(pair[1].color, color::WHITE);
            assert_eq!(pair[1].props().get("label_accent"), Some(pair[0].color.to_string()));
            assert_eq!((pair[0].glow, pair[0].pulse_hz), (0.0, 0.0));
        }
    }

    #[test]
    fn finish_review_compares_the_same_data_and_tints_in_separate_columns() {
        let panels = panel_finishes();
        assert_eq!(panels.len(), 6);
        for (column, finish) in ["color", "white", "black"].into_iter().enumerate() {
            for row in 0..2 {
                let e = &panels[column * 2 + row];
                let reference = &panels[row];
                e.validate().unwrap();
                assert_eq!(e.props().get("panel_finish").as_deref(), Some(finish));
                assert_eq!(e.px, (column as f64 - 1.0) * 9.0);
                assert_eq!((e.sub, &e.values, e.color, e.py, e.pz),
                    (reference.sub, &reference.values, reference.color, reference.py, reference.pz));
            }
        }
    }
}
