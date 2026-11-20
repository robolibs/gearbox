use super::*;
use gearbox_api::wire::draw::rgba;

pub(super) fn contract() -> serde_json::Value {
    let p = Place::default().no_drape().lifetime(7.0);
    let tint = 0xff00b4e9;
    let pose = Pose::yawed(10.0, 20.0, 30.0, 0.5);
    let mut items = w::terrain_marks(
        &p,
        "contract",
        "terrain",
        &[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
        w::TerrainGlyph::Triangle,
        0.4,
        tint,
    );
    items.extend(w::route_stations(
        &p,
        "contract",
        "route",
        &[[0.0; 3], [0.0, 0.0, 3.0], [4.0, 0.0, 3.0]],
        2.0,
        0.4,
        tint,
    ));
    items.extend(w::destination(
        &p,
        "contract",
        "destination",
        pose,
        0.8,
        "Dock",
        tint,
    ));
    items.extend(w::clearance_box(
        &p,
        "contract",
        "clearance",
        pose,
        [2.0, 4.0, 6.0],
        tint,
        "Bounds",
    ));
    items.extend(w::field_volume(
        &p,
        "contract",
        "field",
        pose,
        [2.0, 4.0, 6.0],
        0.5,
        tint,
    ));
    entities_json(&items.into_iter().map(|entity| entity.with_blur(0.6)).collect::<Vec<_>>())
}

pub(super) fn composite_contract() -> serde_json::Value {
    let p = Place::default().no_drape().lifetime(7.0);
    let pose = Pose::yawed(10.0, 20.0, 30.0, 0.5);
    let mut items = w::waypoints(&p, "contract", "waypoints", &[[0.0; 3], [0.0, 0.0, 3.0], [4.0, 0.0, 3.0]], 1);
    items.extend(w::detections_3d(&p, "contract", "detections3d", &[Detection3d {
        pose, size: [2.0, 4.0, 6.0], class: "fixture\nupper".into(), score: 0.82, track: Some(17),
        velocity: None, trail: Vec::new(),
    }], &DetectionStyle::default()));
    items.extend(w::callout(&p, "contract", "callout", [1.0, 2.0, 3.0], [0.4, 0.5, 0.6], "Supplied caption", 0xff00b4e9));
    items.extend(w::frame(&p, "contract", "base", Pose::yawed(1.0, 2.0, 3.0, 0.5), Some([-1.0, -2.0, -3.0]), 0.5, Some("Frame")));
    items.extend(w::detections_2d(&p, "contract", "detections2d", pose, 1.0, 0.8, [640.0, 480.0],
        &[[320.0, 240.0, 100.0, 80.0]], &["fixture".into()], &[0.82], 2.0));
    items.extend(w::detections_3d(&p, "contract", "wire3d", &[Detection3d {
        pose, size: [2.0, 4.0, 6.0], class: "car".into(), score: 0.82, track: Some(17), velocity: None, trail: Vec::new(),
    }], &DetectionStyle { only_edge: true, line_width: -3.0, ..Default::default() }));
    entities_json(&items)
}

pub(super) fn helper_contract() -> serde_json::Value {
    let p = Place::default().no_drape().lifetime(7.0);
    let pose = Pose::yawed(1.0, 2.0, 3.0, 0.5);
    let poses = [pose, Pose::at(4.0, 5.0, 6.0), Pose::at(7.0, 8.0, 9.0)];
    let points = [[0.0, 0.0, 0.0], [2.0, 1.0, 0.5], [4.0, 3.0, 1.0]];
    let outer = vec![[0.0, 0.0, 0.0], [6.0, 0.0, 0.0], [6.0, 6.0, 0.0], [0.0, 6.0, 0.0]];
    let inner = vec![[1.0, 1.0, 0.0], [2.0, 1.0, 0.0], [2.0, 2.0, 0.0], [1.0, 2.0, 0.0]];
    let strips = [points.to_vec(), vec![[1.0, 2.0, 0.0], [3.0, 4.0, 0.0]]];
    let ns = "contract";
    let tint = 0x806f32d1;
    let mut items = vec![
        w::cloud(&p, ns, "cloud", &points, &[0.0, 0.25, 1.0], colormap::TURBO, -1.5, 0.0),
        w::cloud(&p, ns, "cloud-quant", &points, &[0.0, 0.25, 1.0], colormap::TURBO, -1.5, 0.01),
    ];
    items.extend(w::scan(&p, ns, "scan", pose, &[1.0, 2.5, f32::INFINITY, 4.0], -0.5, 0.25, 5.0));
    items.extend(w::range_cone(&p, ns, "range", pose, 1.0, 0.25, 4.0, 2.0));
    items.push(w::camera_frustum(&p, ns, "camera", pose, 1.0, 0.75, 0.25, 5.0));
    items.push(w::pose_array(&p, ns, "poses", &poses, Glyph::Chevron, 0.5));
    items.push(w::particles_colored(&p, ns, "particles", &poses, &[0.0, 0.5, 1.0], 0.1, 0.5, Glyph::Chevron, tint));
    items.extend(w::field(&p, ns, "field-hole", &outer, &[inner.clone()], tint));
    items.push(p.put(E::polygons(ns, "multi-polygon", &[vec![outer.clone(), inner.clone()], vec![points.to_vec()]]))
        .with_style(style::DECAL).with_fill(flag::FILL_BOTH).with_colors(&[tint, 0xff00b4e9]));
    items.push(w::headland(&p, ns, "headland", &outer, &inner));
    items.extend(w::swaths_colored(&p, ns, "swaths", &strips, 2.0, 1, tint));
    items.push(w::turns(&p, ns, "turns", &strips, 2.0));
    items.extend(w::ab_line(&p, ns, "ab", points[0], points[1], 8.0));
    items.push(w::coverage(&p, ns, "coverage", 2, 2, 0.5, Pose::at(1.0, 2.0, 3.0), &[0, 1, 2, 4]));
    items.push(w::implement(&p, ns, "implement", 3.0, 0.6, &[true, false, true]));
    items.push(w::zone(&p, ns, "zone", w::Zone::Stop, &[outer, inner]));
    items.extend(w::graph(&p, ns, "graph", &points, &[(0, 1), (1, 2)], &["A".into(), "B".into(), "C".into()]));
    items.push(w::ghost_trail(&p, ns, "ghost", "rover", &poses, &["steer".into(), "wheel".into()],
        &[vec![0.1, 0.2], vec![0.3, 0.4], vec![0.5, 0.6]], 2, tint));
    for (id, kind) in [("sparkline", panel_kind::SPARKLINE), ("bars", panel_kind::BARS)] {
        items.push(w::panel(&p, ns, id, points[1], kind, &[0.25, -1.5, 2.0], &["A".into(), "B".into(), "C".into()], "Sample"));
    }
    items.push(w::gauge(&p, ns, "gauge", points[1], 42.0, 0.0, 100.0, "Battery", "%"));
    items.push(w::chip(&p, ns, "chip", points[1], "Running", tint));
    items.push(w::table(&p, ns, "table", points[1], &[("Speed".into(), "2 m/s".into()), ("State".into(), "Ready".into())], "Robot"));
    items.push(w::legend(ns, "legend", colormap::VIRIDIS, -2.0, 8.0, "Height", "m"));
    items.push(w::badge(ns, "badge", "rover", "Ready", tint, 2.0));
    items.push(w::voxels(&p, ns, "voxels", &points, 0.2, &[0.25, 0.5, 1.0], colormap::TURBO, true));
    items.extend(w::wrench(&p, ns, "wrench", pose, [2.0, -1.0, 3.0], [1.0, 2.0, 0.5], 0.1));
    items.extend(w::contact(&p, ns, "contact", points[1], [1.0, 2.0, 3.0], 8.0, 0.05));
    items.extend(w::compass_colored(&p, ns, "compass", points[1], 4.0, tint));
    items.extend(w::angle_colored(&p, ns, "angle", points[1], -0.5, 1.0, 2.0, tint));
    items.push(w::ruler(&p, ns, "ruler", points[0], points[2], 0.5));
    items.push(w::scale_bar(&p, ns, "scale", points[1], 5.0));
    items.extend(w::covariance(&p, ns, "covariance", pose, &[0.25, 0.0, 0.0, 1.0], &[1.0, 2.0], tint).unwrap());
    items.extend(w::covariance(&p, ns, "covariance3", pose, &[0.25, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.5], &[1.0, 2.0], tint).unwrap());
    let mut six = [0.0; 36];
    (six[0], six[7], six[14], six[35]) = (0.25, 1.0, 0.5, 0.09);
    items.extend(w::covariance(&p, ns, "covariance6", pose, &six, &[1.0, 2.0], tint).unwrap());
    items.extend(w::odometry(&p, ns, "odometry", pose, [1.0, 2.0, 0.0], &[0.25, 0.0, 0.0, 1.0]));
    items.extend(w::odometry(&p, ns, "odometry-omitted", pose, [0.0; 3], &[]));
    items.extend(w::odometry(&p, ns, "odometry-empty", pose, [0.0; 3], &[]));
    items.push(w::band(&p, ns, "band", &points, &[0.1, 0.2, 0.3]));
    items.extend(w::rollouts(&p, ns, "rollouts", &[strips[0].clone(), strips[1].clone(), points.to_vec()], &[0.0, 0.5, 1.0], &[false, true, false], 1));
    items.extend(w::path_with_markers(&p, ns, "path", &points, 1.0, Glyph::Arrow));
    items.push(w::tf_tree("gearbox/tf", "rover", "rover"));
    items.push(p.put(E::points(ns, "payload-controls", &points)).with_radii(&[0.1, -2.0, 0.3]).with_colors(&[tint, 1, 0xff00b4e9])
        .with_color(tint).with_color2(0x40cc7733).with_style(style::POINT_GLOW).with_width(-3.0)
        .with_glow(-1.0).with_pulse(0.75).with_flow(0.5, 0.25).with_beam(1.5).with_fade(0.3).with_urgency(0.4)
        .with_opacity(0.75)
        .with_values(&[0.0, 0.5, 1.0]).with_colormap(colormap::VIRIDIS, -1.0, 2.0));
    for (id, mark, radius, tint) in [("ring-hover", w::RingMark::Hover, 3.0, 0), ("ring-selected", w::RingMark::Selected, 3.0, tint),
        ("ring-following", w::RingMark::Following, 3.0, 0), ("ring-small", w::RingMark::Following, 0.4, 0)] {
        items.extend(w::machine_ring(&p, ns, id, points[1], radius, mark, tint));
    }
    entities_json(&items)
}

fn entities_json(items: &[E]) -> serde_json::Value {
    serde_json::Value::Array(items.iter().map(|e| serde_json::json!({
        "ns": e.ns(), "id": e.id(), "op": e.op,
        "header": {"op": e.op, "kind": e.kind, "sub": e.sub, "style": e.style, "flags": e.flags,
            "seq": e.seq, "chunk_offset": e.chunk_offset, "last": e.last,
            "color": e.color, "color2": e.color2, "colormap": e.colormap, "vmin": e.vmin, "vmax": e.vmax,
            "quant": e.quant, "width": e.width, "lifetime_s": e.lifetime_s,
            "px": e.px, "py": e.py, "pz": e.pz, "qw": e.qw, "qx": e.qx, "qy": e.qy, "qz": e.qz,
            "sx": e.sx, "sy": e.sy, "sz": e.sz, "dash": e.dash,
            "glow": e.glow, "pulse_hz": e.pulse_hz, "flow_speed": e.flow_speed,
            "fade": e.fade, "beam_height": e.beam_height, "urgency": e.urgency, "opacity": e.opacity},
        "points": e.points, "instances": e.instances, "colors": e.colors, "radii": e.radii, "offsets": e.offsets,
        "rings": e.rings, "values": e.values, "cells": e.cells, "qpoints": e.qpoints, "ranges": e.ranges,
        "labels": e.label_list(), "props": e.props().iter().into_iter().collect::<std::collections::BTreeMap<_, _>>()
    })).collect())
}

fn note(ns: &str, id: &str, at: [f32; 3], text: &str, tint: u32) -> E {
    Place::default()
        .put(E::text(ns, id, &[at], text))
        .with_style(style::LABEL)
        .with_width(-13.0)
        .with_color(tint)
}

fn extent_caption(place: &Place, ns: &str, id: &str, pose: Pose, height: f32, tint: u32, text: &str) -> E {
    let scale = pose.rotation.iter().fold(0.0_f64, |a, b| a.max(b.abs()));
    let q = pose.rotation.map(|v| v / scale);
    let norm = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    place.put(E::text(&format!("{ns}/{id}"), "label", &[[0.0, 0.0, height * 0.5]], text)
        .with_width(-16.0).with_color(tint).with_pose(pose.position, q.map(|v| v / norm))
        .with_prop("drape_anchor", "true").with_prop("drape_pose_height", "true"))
}

pub(super) fn halo(machine: &str) -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/halo");
    let mut out = vec![title(&ns, "ROBOTICS / shared world drawing language")];
    let route = [[-6.0, 1.5, 0.12], [-3.0, 2.8, 0.12], [0.0, 2.4, 0.12], [3.5, 4.4, 0.12]];
    out.extend(w::route_stations(&p, &ns, "route", &route, 0.8, 0.35, color::PLAN));
    let mut destination = w::destination(&p, &ns, "destination", Pose::yawed(3.5, 4.4, 0.12, 0.2), 0.65, "Inspection point A4", color::GOAL);
    for e in &mut destination {
        if e.id() == "label" {
            *e = e.clone().with_prop("label_category", "DESTINATION").with_prop("label_icon", "destination")
                .with_prop("label_detail", "Fixture pose / 0.65 m radius").with_prop("label_selected", "true");
        }
    }
    out.extend(destination);
    out.extend(w::waypoints(&p, &ns, "waypoints", &[[-5.5, -1.2, 0.12], [-3.8, -0.6, 0.12], [-2.2, -1.0, 0.12]], 1));
    out.extend(w::field_volume(&p, &ns, "field", Pose::yawed(-4.6, 4.8, 1.0, 0.12), [3.0, 1.5, 1.7], 0.5, color::SENSOR));
    out.push(extent_caption(&p, &ns, "bounds", Pose::yawed(-4.6, 4.8, 1.0, 0.12), 1.7, color::SENSOR, "Supplied extent / 3 × 1.5 × 1.7 m"));
    for (id, at, tint, glow) in [
        ("ghost", [-0.9, 0.0, 0.0], color::PLAN, 0.0),
        ("ghost-no-glow", [-0.9, -2.7, 0.0], color::PREDICTION, -1.0),
    ] {
        out.push(w::ghost_trail(&p, &ns, id, machine, &[Pose::at(at[0], at[1], at[2])], &[], &[], 1, tint)
            .with_glow(glow));
    }
    out.push(E::text(&ns, "robot-name", &[[0.0, 0.0, 1.8]], machine)
        .in_frame(machine).with_width(-15.0).with_color(color::WHITE)
        .with_prop("label_category", "ROBOT").with_prop("label_icon", "robot"));
    out.push(note(&ns, "regular-label", [-1.0, -2.6, 1.6], "Ghost / aura off", color::WHITE)
        .with_prop("label_icon", "robot"));
    out.extend(w::callout(&p, &ns, "callout", [0.3, 0.0, 0.5], [1.1, -0.4, 0.8], "Legacy callout", color::PLAN));
    out.extend(w::detections_3d(&p, &ns, "detection", &[Detection3d {
        pose: Pose::yawed(4.0, 0.3, 0.75, 0.25), size: [1.6, 1.0, 1.4], class: "fixture".into(),
        score: 0.82, track: Some(17), velocity: None, trail: Vec::new(),
    }], &DetectionStyle::default()));
    let cells: Vec<_> = (0..144).map(|i| if i / 12 == 5 && (3..9).contains(&(i % 12)) { 255 } else { ((i % 12) * 9) as u8 }).collect();
    out.push(p.put(E::grid(&ns, "scalar-grid", 12, 12, 0.25, GridCells::U8(cells)))
        .at_yaw([-6.2, -5.3, 0.08], 0.0).with_colormap(colormap::VIRIDIS, 0.0, 100.0)
        .with_prop("unknown", "255").with_spacing(20.0));
    out.push(note(&ns, "grid-label", [-4.7, -5.6, 0.3], "Scalar fixture / unknown cells omitted", color::WHITE)
        .with_prop("label_category", "VIRIDIS 0–100").with_prop("label_icon", "sensor"));
    out.push(w::ruler(&p, &ns, "ruler", [-2.5, -5.0, 0.12], [1.5, -5.0, 0.12], 0.5));
    out.push(w::table(&p, &ns, "inspection-table", [5.3, -3.7, 1.3], &[
        ("Source".into(), "Deterministic fixture".into()),
        ("Grid".into(), "0–100 / unknown 255".into()),
        ("Bounds".into(), "3 × 1.5 × 1.7 m".into()),
    ], "INSPECTION"));
    out.push(p.put(E::lines(&ns, "explicit-magenta", &[[1.6, -6.0, 0.12], [5.8, -6.0, 0.12]]))
        .with_style(style::RIBBON_FLOW).with_color(color::MAGENTA).with_width(0.12).with_glow(-1.0).with_pulse(-1.0).with_flow(-1.0, 0.0)
        .labelled(&["Explicit magenta / static / glow off"]));
    out
}

pub(super) fn label_contrast() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/label-contrast");
    let mut out = vec![title(&ns, "LABEL CONTRAST / supplied colors, tight glyph edges")];
    for (i, (text, tint)) in [
        ("Black / supplied RGB", rgba(0, 0, 0, 255)), ("Pale / supplied RGB", color::WHITE),
        ("Dark blue / supplied RGB", color::SENSOR), ("Yellow / supplied RGB", color::YELLOW),
        ("Dark red / supplied RGB", rgba(70, 12, 24, 255)), ("Gray / supplied RGB", rgba(180, 180, 180, 255)),
        ("Dark green / supplied RGB", rgba(10, 65, 35, 255)), ("Magenta / supplied RGB", color::MAGENTA),
    ].into_iter().enumerate() {
        out.push(p.put(E::text(&ns, &format!("ink-{i}"), &[[if i % 2 == 0 { -4.0 } else { 4.0 }, 6.0 - (i / 2) as f32 * 2.5, 1.0]], text))
            .with_width(-16.0).with_color(tint));
    }
    for (id, x, accent, caption) in [
        ("category-clear", -4.0, 0, "Category alpha 0 / no outline"),
        ("category-half", 4.0, rgba(0, 114, 178, 128), "Category alpha 128 / supplied tint"),
    ] {
        out.push(p.put(E::text(&ns, id, &[[x, -4.0, 1.0]], caption)).with_width(-16.0).with_color(color::WHITE)
            .with_prop("label_category", "CATEGORY").with_prop("label_accent", &accent.to_string()));
    }
    for (id, x, alpha, caption) in [
        ("fade-opaque", -4.0, 255, "Gray 112 / alpha 255"), ("fade-half", 4.0, 128, "Gray 112 / alpha 128"),
    ] {
        out.push(p.put(E::text(&ns, id, &[[x, -6.5, 1.0]], caption)).with_width(-16.0).with_color(rgba(112, 112, 112, alpha)));
    }
    out
}

pub(super) fn survey() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/survey");
    let mut out = vec![title(&ns, "SURVEY / caller-defined samples and route")];
    let families = [
        (
            w::TerrainGlyph::Square,
            color::SENSOR,
            -5.0,
            -2.5,
            "sample-a",
        ),
        (
            w::TerrainGlyph::Triangle,
            color::PREDICTION,
            0.0,
            -2.5,
            "sample-b",
        ),
        (w::TerrainGlyph::Cross, color::DANGER, 4.0, -2.5, "sample-c"),
        (
            w::TerrainGlyph::Diamond,
            color::WHITE,
            -4.5,
            2.3,
            "sample-d",
        ),
    ];
    for (glyph, tint, x, y, name) in families {
        let points: Vec<_> = (0..35)
            .map(|i| [x + (i % 7) as f32 * 0.47, y + (i / 7) as f32 * 0.52, 0.16])
            .collect();
        out.extend(w::terrain_marks(&p, &ns, name, &points, glyph, 0.22, tint));
    }
    let route = [
        [-6.0, -5.5, 0.15],
        [-2.0, -4.0, 0.15],
        [-1.0, -0.2, 0.15],
        [1.0, 2.4, 0.15],
        [5.0, 5.0, 0.15],
    ];
    out.extend(w::route_stations(
        &p,
        &ns,
        "route",
        &route,
        0.85,
        0.45,
        color::PLAN,
    ));
    out.extend(w::destination(
        &p,
        &ns,
        "dock",
        Pose::yawed(5.0, 5.0, 0.15, 0.35),
        0.8,
        "DOCK / A-04",
        color::GOAL,
    ));
    out.extend(w::clearance_box(
        &p,
        &ns,
        "bounds",
        Pose::yawed(5.2, -1.3, 0.9, 0.2),
        [3.5, 2.9, 1.6],
        color::DANGER,
        "SUPPLIED BOUNDS",
    ));
    out.push(note(
        &ns,
        "classes",
        [-3.8, 5.5, 0.5],
        "SAMPLE CLASSES / fixture values, not inferred safety",
        color::WHITE,
    ));
    out
}

pub(super) fn labels() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/labels");
    let mut out = vec![title(&ns, "World labels / anchored to scene data")];
    for (id, at, text, tint) in [
        ("dock", [-4.0, -2.5, 0.4], "Charging station A4", color::PLAN),
        ("survey", [2.5, -0.5, 0.4], "Survey sample 028", color::SENSOR),
        ("inspection", [-2.0, 3.0, 0.4], "Inspection waypoint B2", color::GOAL),
    ] {
        out.push(p.put(E::points(&ns, id, &[at]))
            .with_style(style::POINT_GLOW).with_width(0.12).with_glow(0.4)
            .with_color(tint).labelled(&[text]));
        out.extend(w::terrain_marks(&p, &ns, &format!("{id}-mark"), &[[at[0], at[1], 0.15]], w::TerrainGlyph::Diamond, 0.7, tint));
    }
    out.push(p.put(E::text(&ns, "centred", &[[4.0, 4.0, 0.7]], "Terrain observation / lidar"))
        .with_width(-16.0));
    out
}

pub(super) fn spatial() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/spatial");
    let mut out = vec![title(
        &ns,
        "SPATIAL / rectangular fields and measured bounds",
    )];
    out.extend(w::field_volume(
        &p,
        &ns,
        "coverage",
        Pose::yawed(-3.7, -0.2, 1.55, 0.2),
        [4.8, 3.0, 2.8],
        0.75,
        color::SENSOR,
    ));
    out.push(extent_caption(
        &p,
        &ns,
        "coverage-bounds",
        Pose::yawed(-3.7, -0.2, 1.55, 0.2),
        2.8,
        color::SENSOR,
        "4.8 x 3.0 x 2.8 m",
    ));
    out.extend(w::field_volume(
        &p,
        &ns,
        "curtain",
        Pose::yawed(3.8, -0.2, 1.55, -0.3),
        [4.6, 0.25, 2.8],
        0.7,
        color::GOAL,
    ));
    out.push(extent_caption(
        &p,
        &ns,
        "curtain-bounds",
        Pose::yawed(3.8, -0.2, 1.55, -0.3),
        2.8,
        color::GOAL,
        "BOUNDED FIELD / not an obstacle",
    ));
    out.extend(w::destination(
        &p,
        &ns,
        "inspection",
        Pose::yawed(0.0, 5.0, 0.15, -0.35),
        0.7,
        "INSPECTION / B-02",
        color::PLAN,
    ));
    out.extend(w::route_stations(
        &p,
        &ns,
        "approach",
        &[
            [-5.5, -5.0, 0.15],
            [-1.0, -3.8, 0.15],
            [1.0, 0.9, 0.15],
            [0.0, 5.0, 0.15],
        ],
        0.9,
        0.4,
        color::PLAN,
    ));
    out
}

pub(super) fn controls() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/world-controls");
    let mut out = vec![title(&ns, "WORLD CONTROLS / motion, glow, alpha, rotation")];
    for (i, (name, glow, pulse, tint)) in [
        ("active", 0.0, 0.0, color::PLAN),
        ("no-glow", -1.0, 0.0, color::PLAN),
        ("no-motion", 0.0, -1.0, color::PLAN),
        ("alpha-zero", 0.0, 0.0, color::with_alpha(color::PLAN, 0.0)),
    ]
    .into_iter()
    .enumerate()
    {
        let x = (i % 2) as f64 * 6.0 - 3.0;
        let y = (i / 2) as f64 * 5.0 - 2.5;
        let mut items = w::field_volume(
            &p,
            &ns,
            name,
            Pose::yawed(x, y, 1.3, 0.37),
            [3.0, 1.6, 2.3],
            0.6,
            tint,
        );
        for e in &mut items {
            e.glow = glow;
            e.pulse_hz = pulse;
        }
        out.extend(items);
        out.push(note(
            &ns,
            &format!("label-{i}"),
            [x as f32, y as f32, 2.8],
            name,
            color::WHITE,
        ));
    }
    let raw = Place::frame("world").no_drape();
    out.extend(w::clearance_box(
        &raw,
        &ns,
        "absolute",
        Pose::yawed(0.0, -6.0, 1.4, 0.7),
        [2.0, 0.8, 1.2],
        color::PREDICTION,
        "ABSOLUTE Z",
    ));
    out
}

pub(super) fn dense() -> Vec<E> {
    let p = Place::default();
    let ns = format!("{NS}/world-dense");
    let mut out = vec![title(&ns, "DENSE WORLD / 2,304 caller-supplied glyphs")];
    for (group, (glyph, tint)) in [
        (w::TerrainGlyph::Square, color::SENSOR),
        (w::TerrainGlyph::Triangle, color::PREDICTION),
        (w::TerrainGlyph::Cross, color::DANGER),
        (w::TerrainGlyph::Diamond, color::WHITE),
    ]
    .into_iter()
    .enumerate()
    {
        let points: Vec<_> = (0..576)
            .map(|i| {
                [
                    (group % 2) as f32 * 8.0 - 7.5 + (i % 24) as f32 * 0.3,
                    (group / 2) as f32 * 8.0 - 7.5 + (i / 24) as f32 * 0.3,
                    0.16,
                ]
            })
            .collect();
        out.extend(w::terrain_marks(
            &p,
            &ns,
            &format!("class-{group}"),
            &points,
            glyph,
            0.15,
            tint,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_scenes_have_no_hud_or_forced_visibility() {
        for entity in survey()
            .into_iter()
            .chain(spatial())
            .chain(controls())
            .chain(dense())
        {
            entity.validate().unwrap();
            assert_eq!(entity.flags & flag::ON_TOP, 0);
            assert_ne!(entity.kind, gearbox_api::draw_kind::PANEL);
            assert_ne!(entity.kind, gearbox_api::draw_kind::INTERACT);
            assert_eq!(
                entity.props().get("frame").as_deref().unwrap_or("world"),
                "world"
            );
        }
    }

    #[test]
    fn field_controls_keep_explicit_off_values() {
        let items = controls();
        let field = |name: &str| {
            items
                .iter()
                .find(|e| e.ns().ends_with(name) && e.style == style::FIELD)
                .unwrap()
        };
        assert_eq!(field("no-glow").glow, -1.0);
        assert_eq!(field("no-motion").pulse_hz, -1.0);
        assert_eq!(field("alpha-zero").color >> 24, 0);
    }

    #[test]
    fn contrast_scene_keeps_explicit_foregrounds_and_category_alpha() {
        let items = label_contrast();
        assert_eq!(items.len(), 13);
        for e in &items {
            e.validate().unwrap();
            assert_eq!(e.flags & flag::ON_TOP, 0);
            assert_ne!(e.props().get("label_neutral_identity").as_deref(), Some("true"));
        }
        let item = |id| items.iter().find(|e| e.id() == id).unwrap();
        assert_eq!(item("ink-0").color, rgba(0, 0, 0, 255));
        assert_eq!(item("ink-2").color, color::SENSOR);
        assert_eq!(item("category-clear").props().get("label_accent").as_deref(), Some("0"));
        assert_eq!(item("category-half").props().get("label_accent").unwrap().parse::<u32>().unwrap() >> 24, 128);
        assert_eq!(item("fade-opaque").color, rgba(112, 112, 112, 255));
        assert_eq!(item("fade-half").color, rgba(112, 112, 112, 128));
    }

    #[test]
    fn mixed_fields_have_one_corner_owner_and_preserve_extent_captions() {
        let p = Place::default();
        for (scene, field, bounds, pose, size, spacing, tint, caption) in [
            ("halo", "field", "bounds", Pose::yawed(-4.6, 4.8, 1.0, 0.12), [3.0, 1.5, 1.7], 0.5, color::SENSOR, "Supplied extent / 3 × 1.5 × 1.7 m"),
            ("spatial", "coverage", "coverage-bounds", Pose::yawed(-3.7, -0.2, 1.55, 0.2), [4.8, 3.0, 2.8], 0.75, color::SENSOR, "4.8 x 3.0 x 2.8 m"),
            ("spatial", "curtain", "curtain-bounds", Pose::yawed(3.8, -0.2, 1.55, -0.3), [4.6, 0.25, 2.8], 0.7, color::GOAL, "BOUNDED FIELD / not an obstacle"),
        ] {
            let ns = format!("{NS}/{scene}");
            let items = if scene == "halo" { halo("fixture_robot") } else { spatial() };
            let retained: Vec<_> = items.iter().filter(|e| e.ns() == format!("{ns}/{bounds}")).cloned().collect();
            assert_eq!(retained.len(), 1, "{scene}/{bounds}: duplicate geometry over FIELD corners");
            let expected = w::clearance_box(&p, &ns, bounds, pose, size, tint, caption).pop().unwrap();
            assert_eq!(entities_json(&retained), entities_json(&[expected]));
            let volume: Vec<_> = items.iter().filter(|e| e.ns() == format!("{ns}/{field}")).cloned().collect();
            assert_eq!(entities_json(&volume), entities_json(&w::field_volume(&p, &ns, field, pose, size, spacing, tint)));
        }
    }

    #[test]
    fn halo_scene_has_world_annotations_data_and_explicit_static_control() {
        let items = halo("fixture_robot");
        for e in &items {
            e.validate().unwrap();
            assert_eq!(e.flags & flag::ON_TOP, 0);
        }
        assert!(items.iter().any(|e| e.props().get("label_selected").as_deref() == Some("true")));
        assert!(items.iter().any(|e| e.id() == "regular-label" && e.props().get("label_selected").is_none()));
        let grid = items.iter().find(|e| e.id() == "scalar-grid").unwrap();
        assert_eq!(grid.colormap, colormap::VIRIDIS);
        assert_eq!((grid.vmin, grid.vmax), (0.0, 100.0));
        assert_eq!(grid.cells.iter().filter(|v| **v == 255).count(), 6);
        let explicit = items.iter().find(|e| e.id() == "explicit-magenta").unwrap();
        assert_eq!((explicit.color, explicit.glow), (color::MAGENTA, -1.0));
        assert_eq!((explicit.pulse_hz, explicit.flow_speed), (-1.0, -1.0));
        assert!(items.iter().any(|e| e.kind == gearbox_api::draw_kind::GHOST));
        let panel = items.iter().find(|e| e.kind == gearbox_api::draw_kind::PANEL).unwrap();
        assert_eq!(panel.sub, panel_kind::TABLE);
        assert!(panel.px > 5.0 && panel.py < -3.0 && panel.pz > 1.0);
    }
}
