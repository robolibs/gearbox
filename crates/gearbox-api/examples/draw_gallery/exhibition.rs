use super::*;

const PITCH: f64 = 18.0;
const HALF: f64 = 5.0;

struct Bay {
    section: &'static str,
    name: &'static str,
    helpers: Vec<&'static str>,
    items: Vec<E>,
    action: Option<&'static str>,
}

impl Bay {
    fn new(section: &'static str, name: &'static str, helpers: &[&'static str], make: impl FnOnce(&str) -> Vec<E>) -> Self {
        let ns = format!("{NS}/exhibition/{section}/{name}");
        Self { section, name, helpers: helpers.to_vec(), items: make(&ns), action: None }
    }
}

fn bays(machine: &str) -> Vec<Bay> {
    let p = Place::default();
    let tint = color::PLAN;
    let at = Pose::at(0.0, 0.0, 0.2);
    let path = [[-2.0, -1.0, 0.15], [0.0, 0.8, 0.15], [2.0, -0.5, 0.15]];
    let poses = [Pose::yawed(-1.5, 0.0, 0.2, 0.0), Pose::yawed(0.0, 0.0, 0.2, 0.4), Pose::yawed(1.5, 0.0, 0.2, 0.8)];
    let outer = rect(-2.0, -2.0, 4.0, 4.0);
    let inner = rect(-1.2, -1.2, 2.4, 2.4);
    let mut out = vec![Bay::new("robot", "live-frame", &["badge", "tf_tree"], |ns| vec![
        w::badge(ns, "badge", machine, "Robot fixture", tint, 2.0), w::tf_tree(ns, "tf", machine),
    ])];
    out.push(Bay::new("robot", "ghost-trail", &["ghost_trail"], |ns| vec![
        w::ghost_trail(&p, ns, "ghost", machine, &poses, &[], &[], 1, tint),
    ]));
    out.push(Bay::new("robot", "frame", &["frame"], |ns| w::frame(&p, ns, "frame", at, Some([-2.0, 0.0, 0.2]), 1.2, Some("Frame"))));
    for (name, glyph) in [("poses-chevron", Glyph::Chevron), ("poses-arrow", Glyph::Arrow), ("poses-axes", Glyph::Axes)] {
        out.push(Bay::new("robot", name, &["pose_array"], |ns| vec![w::pose_array(&p, ns, "poses", &poses, glyph, 0.8)]));
    }
    out.push(Bay::new("robot", "odometry", &["odometry"], |ns| w::odometry(&p, ns, "odom", at, [1.5, 0.4, 0.0], &[0.3, 0.0, 0.0, 0.2])));
    out.push(Bay::new("robot", "wrench", &["wrench"], |ns| w::wrench(&p, ns, "wrench", at, [6.0, 3.0, 2.0], [0.0, 0.0, 3.0], 0.25)));
    out.push(Bay::new("robot", "contact", &["contact"], |ns| w::contact(&p, ns, "contact", [0.0, 0.0, 0.15], [0.0, 0.0, 1.0], 4.0, 0.4)));
    out.push(Bay::new("robot", "implement", &["implement"], |ns| vec![w::implement(&p, ns, "implement", 4.0, 1.0, &[true, false, true, true])]));

    let source = variants::styles(machine);
    for look in style::PLAIN..=style::FIELD {
        let mut e = source[(look as usize - 1) * 2 + 1].clone();
        let i = look - 1;
        e.px -= ((i % 5) as f32 * 3.4 - 6.8) as f64;
        e.py -= (5.5 - (i / 5) as f32 * 3.4) as f64;
        e.pulse_hz = 0.0;
        e.flow_speed = 0.0;
        out.push(Bay::new("styles", style::name(look), &[], |ns| vec![e.with_prop("ns", ns)]));
    }
    for shape in 0..=shape::SECTOR {
        let tint = [color::CYAN, color::MAGENTA, color::GREEN, color::ORANGE, color::WHITE][shape as usize % 5];
        let size = match shape {
            shape::FRUSTUM => [1.0, 0.7, 2.5], shape::SECTOR => [2.0, 1.8, 0.3],
            shape::ELLIPSE | shape::RECT => [2.5, 1.8, 0.0], _ => [2.0, 1.6, 1.6],
        };
        out.push(Bay::new("shapes", shape::name(shape), &[], |ns| vec![p.put(E::shapes(ns, "shape", shape, &[Instance::at([0.0, 0.0, 1.0], size)]))
            .with_style(style::HOLO).with_fill(flag::FILL_BOTH).with_color(tint)]));
    }
    for (name, fill) in [("solid", flag::FILL_SOLID), ("wire", flag::FILL_WIRE), ("both", flag::FILL_BOTH)] {
        out.push(Bay::new("shapes", name, &[], |ns| vec![p.put(E::shapes(ns, "fill", shape::BOX, &[Instance::at([0.0, 0.0, 1.0], [2.0; 3])]))
            .with_style(style::PLAIN).with_fill(fill).with_color(tint).with_width(0.04)]));
    }
    for (name, sub) in [("point-sphere", point_style::SPHERE), ("point-disc", point_style::DISC), ("point-square", w::POINT_SQUARE)] {
        out.push(Bay::new("primitives", name, &[], |ns| vec![p.put(E::points(ns, "points", &[[-1.0, 0.0, 0.6], [1.0, 0.0, 0.6]]))
            .with_style(style::PLAIN).with_sub(sub).with_width(0.65).with_color(tint)]));
    }
    for (name, mode) in [("line-strip", line_mode::STRIP), ("line-list", line_mode::LIST), ("line-loop", line_mode::LOOP)] {
        out.push(Bay::new("primitives", name, &[], |ns| vec![p.put(E::lines(ns, "line", &outer)).with_sub(mode).with_color(tint).with_width(0.06)]));
    }
    out.push(Bay::new("primitives", "triangles", &[], |ns| vec![p.put(E::triangles(ns, "triangles", &[[-1.8, -1.0, 0.1], [1.8, -1.0, 0.1], [0.0, 1.8, 0.1]]))
        .with_color(tint)]));
    out.push(Bay::new("primitives", "polygon-hole", &[], |ns| vec![p.put(E::polygon(ns, "polygon", &[outer.clone(), inner.clone()]))
        .with_fill(flag::FILL_BOTH).with_color(tint)]));
    out.push(Bay::new("primitives", "mesh", &[], |ns| vec![p.put(E::mesh(ns, "mesh", &format!("{}/../../tests/fixtures/draw_vfx/faceted.obj", env!("CARGO_MANIFEST_DIR"))))
        .with_color(tint)]));

    out.push(Bay::new("navigation", "waypoints", &["waypoints"], |ns| w::waypoints(&p, ns, "route", &path, 1)));
    out.push(Bay::new("navigation", "path-markers", &["path_with_markers"], |ns| w::path_with_markers(&p, ns, "path", &path, 0.7, Glyph::Chevron)));
    out.push(Bay::new("navigation", "route-stations", &["route_stations"], |ns| w::route_stations(&p, ns, "route", &path, 0.7, 0.4, tint)));
    out.push(Bay::new("navigation", "destination", &["destination"], |ns| w::destination(&p, ns, "goal", at, 0.9, "Destination", tint)));
    out.push(Bay::new("navigation", "clearance", &["clearance_box"], |ns| w::clearance_box(&p, ns, "bounds", Pose::at(0.0, 0.0, 1.1), [3.0, 2.0, 2.0], tint, "Supplied bounds")));
    out.push(Bay::new("navigation", "field-volume", &["field_volume"], |ns| w::field_volume(&p, ns, "volume", Pose::at(0.0, 0.0, 1.2), [3.5, 2.5, 2.2], 0.6, tint)));
    for glyph in [w::TerrainGlyph::Square, w::TerrainGlyph::Cross, w::TerrainGlyph::Triangle, w::TerrainGlyph::Diamond] {
        out.push(Bay::new("navigation", glyph.name(), &["terrain_marks"], |ns| w::terrain_marks(&p, ns, "marks", &path, glyph, 0.7, tint)));
    }
    out.push(Bay::new("navigation", "graph", &["graph"], |ns| w::graph(&p, ns, "graph", &path, &[(0, 1), (1, 2), (2, 0)], &[])));
    out.push(Bay::new("navigation", "rollouts", &["rollouts"], |ns| w::rollouts(&p, ns, "rollouts",
        &(0..12).map(|k| (0..16).map(|i| [-2.0 + i as f32 * 0.25, (k as f32 - 5.5) * 0.025 * (i as f32 * 0.25).powi(2), 0.12]).collect()).collect::<Vec<_>>(),
        &(0..12).map(|i| i as f32).collect::<Vec<_>>(), &[false; 12], 1)));
    out.push(Bay::new("navigation", "uncertainty-band", &["band"], |ns| vec![w::band(&p, ns, "band", &path, &[0.3])]));

    for (name, cov) in [("covariance-2d", vec![0.35, 0.08, 0.08, 0.2]), ("covariance-3d", vec![0.35, 0.0, 0.0, 0.0, 0.2, 0.0, 0.0, 0.0, 0.1])] {
        out.push(Bay::new("perception", name, &["covariance"], |ns| w::covariance(&p, ns, "covariance", Pose::at(0.0, 0.0, 1.0), &cov, &[1.0, 2.0], tint).unwrap()));
    }
    let mut cov6 = vec![0.0; 36];
    for (i, value) in [(0, 0.35), (7, 0.2), (14, 0.1), (35, 0.08)] { cov6[i] = value; }
    out.push(Bay::new("perception", "covariance-6d", &["covariance"], |ns| w::covariance(&p, ns, "covariance", at, &cov6, &[1.0, 2.0], tint).unwrap()));
    out.push(Bay::new("perception", "particles", &["particles"], |ns| vec![w::particles(&p, ns, "particles", &poses, &[0.3, 0.7, 1.0], 0.3, 1.0, Glyph::Chevron)]));
    out.push(Bay::new("perception", "cloud", &["cloud"], |ns| {
        let points: Vec<_> = (0..400).map(|i| [-2.0 + (i % 20) as f32 * 0.2, -2.0 + (i / 20) as f32 * 0.2, 0.2 + (i as f32 * 0.15).sin().abs()]).collect();
        let values: Vec<_> = points.iter().map(|p| p[2]).collect();
        vec![w::cloud(&p, ns, "cloud", &points, &values, colormap::VIRIDIS, -2.0, 0.01)]
    }));
    out.push(Bay::new("perception", "scan", &["scan"], |ns| w::scan(&p, ns, "scan", at, &[2.5; 61], -1.2, 0.04, 3.0)));
    out.push(Bay::new("perception", "range-cone", &["range_cone"], |ns| w::range_cone(&p, ns, "range", at, 0.8, 0.2, 3.0, 2.0)));
    out.push(Bay::new("perception", "camera-frustum", &["camera_frustum", "camera_fov"], |ns| {
        let (h, v) = w::camera_fov(500.0, 500.0, 640.0, 480.0);
        vec![w::camera_frustum(&p, ns, "camera", at, h, v, 0.2, 3.0)]
    }));
    out.push(Bay::new("perception", "detection-3d", &["detections_3d", "class_color"], |ns| w::detections_3d(&p, ns, "detections", &[Detection3d {
        pose: Pose::yawed(0.0, 0.0, 1.0, 0.3), size: [2.0, 1.4, 1.8], class: "fixture".into(), score: 0.82, track: Some(17),
        velocity: Some([1.0, 0.0, 0.0]), trail: vec![[-2.0, -0.4, 0.2], [-1.0, -0.1, 0.2], [0.0, 0.0, 0.2]],
    }], &DetectionStyle::default())));
    out.push(Bay::new("perception", "detection-2d", &["detections_2d"], |ns| w::detections_2d(&p, ns, "detections", at, 1.0, 0.8,
        [640.0, 480.0], &[[320.0, 240.0, 240.0, 180.0]], &["fixture".into()], &[0.82], 2.5)));
    out.push(Bay::new("perception", "voxels", &["voxels"], |ns| vec![w::voxels(&p, ns, "voxels", &path, 0.7, &[0.2, 0.6, 1.0], colormap::VIRIDIS, false)]));

    out.push(Bay::new("agriculture", "field", &["field"], |ns| w::field(&p, ns, "field", &outer, &[], tint)));
    out.push(Bay::new("agriculture", "headland", &["headland"], |ns| vec![w::headland(&p, ns, "headland", &outer, &inner)]));
    let swaths = vec![vec![[-2.0, -1.4, 0.1], [2.0, -1.4, 0.1]], vec![[-2.0, 0.0, 0.1], [2.0, 0.0, 0.1]], vec![[-2.0, 1.4, 0.1], [2.0, 1.4, 0.1]]];
    out.push(Bay::new("agriculture", "swaths", &["swaths"], |ns| w::swaths(&p, ns, "swaths", &swaths, 0.65, 1)));
    out.push(Bay::new("agriculture", "turns", &["turns"], |ns| vec![w::turns(&p, ns, "turns", &[path.to_vec()], 0.6)]));
    out.push(Bay::new("agriculture", "ab-line", &["ab_line"], |ns| w::ab_line(&p, ns, "ab", [-2.0, 0.0, 0.1], [2.0, 0.0, 0.1], 0.5)));
    out.push(Bay::new("agriculture", "coverage", &["coverage"], |ns| vec![w::coverage(&p, ns, "coverage", 8, 8, 0.5, Pose::at(-2.0, -2.0, 0.1),
        &(0..64).map(|i| (i % 4 + 1) as u8).collect::<Vec<_>>())]));
    for (name, zone) in [("keepout", w::Zone::Keepout), ("slow", w::Zone::Slow), ("stop", w::Zone::Stop)] {
        out.push(Bay::new("agriculture", name, &["zone"], |ns| vec![w::zone(&p, ns, "zone", zone, &[outer.clone()])]));
    }

    out.push(Bay::new("annotations", "measurement-rule", &["ruler", "scale_bar"], |ns| vec![w::scale_bar(&p, ns, "scale", [-2.0, 0.0, 0.2], 4.0)]));
    out.push(Bay::new("annotations", "compass", &["compass"], |ns| w::compass(&p, ns, "compass", [0.0, 0.0, 0.1], 1.5)));
    out.push(Bay::new("annotations", "angle", &["angle"], |ns| w::angle(&p, ns, "angle", [0.0, 0.0, 0.1], 0.0, 1.0, 2.0)));
    out.push(Bay::new("annotations", "callout", &["callout"], |ns| w::callout(&p, ns, "callout", [-1.0, 0.0, 0.2], [2.0, 0.0, 1.0], "Supplied caption", tint)));
    for (name, selected) in [("label-regular", false), ("label-selected", true)] {
        out.push(Bay::new("annotations", name, &[], |ns| vec![p.put(E::text(ns, "text", &[[0.0, 0.0, 1.0]], "Inspection fixture"))
            .with_width(-16.0).with_color(color::WHITE).with_prop("label_accent", &tint.to_string()).with_prop("label_category", "ANNOTATION")
            .with_prop("label_detail", "Supplied detail / 0.4 m").with_prop("label_icon", "measurement").with_prop("label_selected", if selected { "true" } else { "false" })]));
    }
    for icon in ["robot", "sensor", "destination", "measurement", "obstacle", "frame"] {
        out.push(Bay::new("annotations", icon, &[], |ns| vec![p.put(E::text(ns, "icon", &[[0.0, 0.0, 0.7]], icon))
            .with_color(color::WHITE).with_width(-16.0).with_prop("label_icon", icon).with_prop("label_accent", &tint.to_string())]));
    }

    for kind in 0..=panel_kind::LEGEND {
        let name = ["sparkline", "bars", "gauge", "chip", "table", "legend"][kind as usize];
        let finish = ["black", "white", "color"][kind as usize % 3];
        let tint = [color::GREEN, color::MAGENTA, color::ORANGE, color::YELLOW, color::CYAN, color::WHITE][kind as usize];
        out.push(Bay::new("panels", name, &[match kind { panel_kind::GAUGE => "gauge", panel_kind::CHIP => "chip", panel_kind::TABLE => "table", panel_kind::LEGEND => "legend", _ => "panel" }], |ns| {
            let e = match kind {
                panel_kind::GAUGE => w::gauge(&p, ns, "gauge", [0.0, 0.0, 0.5], 0.65, 0.0, 1.0, "Fixture value", "m"),
                panel_kind::CHIP => w::chip(&p, ns, "chip", [0.0, 0.0, 0.5], "Fixture caption", tint),
                panel_kind::TABLE => w::table(&p, ns, "table", [0.0, 0.0, 0.5], &[("Source".into(), "Fixture".into()), ("Value".into(), "0.65 m".into())], "Supplied data"),
                panel_kind::LEGEND => p.put(w::legend(ns, "legend", colormap::VIRIDIS, 0.0, 1.0, "Scalar scale", "m"))
                    .at_yaw([0.0, 0.0, 0.5], 0.0).with_prop("world_anchor", "true"),
                _ => w::panel(&p, ns, "panel", [0.0, 0.0, 0.5], kind, &[0.2, 0.8, 0.4, 0.65], &["A".into(), "B".into(), "C".into(), "D".into()], "Fixture samples"),
            };
            vec![e.with_color(tint).with_prop("panel_finish", finish)]
        }));
    }
    for map in 1..=colormap::GREY {
        out.push(Bay::new("data", colormap::name(map), &[], |ns| vec![p.put(E::grid(ns, "grid", 8, 8, 0.5,
            GridCells::U8((0..64).map(|i| if i == 27 { 255 } else { (i * 100 / 63) as u8 }).collect())))
            .at_yaw([-2.0, -2.0, 0.1], 0.0).with_colormap(map, 0.0, 100.0).with_spacing(20.0).with_prop("unknown", "255")]));
    }
    for (name, cells) in [("grid-u16", GridCells::U16((0..16).map(|i| i * 100).collect())), ("grid-f32", GridCells::F32((0..16).map(|i| i as f32 * 0.1).collect()))] {
        out.push(Bay::new("data", name, &[], |ns| vec![p.put(E::grid(ns, "grid", 4, 4, 0.8, cells)).at_yaw([-1.6, -1.6, 0.1], 0.0)
            .with_colormap(colormap::VIRIDIS, 0.0, 0.0)]));
    }
    for action in ["handle", "pick-point", "pick-pose", "step"] {
        let mut bay = Bay::new("interactions", action, &[], |ns| {
            let mut items = w::clearance_box(&p, ns, "target", Pose::at(0.0, 0.0, 0.6), [3.0, 2.0, 1.0], tint, "Activate from manifest");
            if action == "handle" {
                items.push(E::interact(ns, "request", interact_kind::HANDLE).at_yaw([0.0, 0.0, 0.6], 0.0).with_prop("controls", "move,rotate").with_prop("menu", "Inspect,Reset"));
            }
            items
        });
        bay.action = Some(action);
        out.push(bay);
    }
    for (name, glow, pulse, tint) in [("aura-off-energy-on", -1.0, 0.5, tint), ("motion-off", 0.0, -1.0, tint),
        ("alpha-zero", 0.0, 0.5, 1), ("explicit-magenta", 0.0, 0.5, color::MAGENTA)] {
        out.push(Bay::new("controls", name, &["field_volume"], |ns| w::field_volume(&p, ns, "field", Pose::at(0.0, 0.0, 1.0),
            [3.0, 2.0, 1.8], 0.5, tint).into_iter().map(|e| e.with_glow(glow).with_pulse(pulse)).collect()));
    }
    let represented: std::collections::BTreeSet<_> = out.iter().filter(|bay| bay.section != "styles")
        .flat_map(|bay| bay.items.iter().map(|e| e.style)).collect();
    out.retain(|bay| bay.section != "styles" || bay.items.iter().any(|e| !represented.contains(&e.style)));
    out
}

fn centres(count: usize) -> Vec<[f64; 2]> {
    let mut cells = vec![[0.0, 0.0]];
    let mut r = 1_i32;
    while cells.len() < count {
        for x in -r..r { cells.push([x as f64 * PITCH, -r as f64 * PITCH]); }
        for y in -r..r { cells.push([r as f64 * PITCH, y as f64 * PITCH]); }
        for x in (-r + 1..=r).rev() { cells.push([x as f64 * PITCH, r as f64 * PITCH]); }
        for y in (-r + 1..=r).rev() { cells.push([-r as f64 * PITCH, y as f64 * PITCH]); }
        r += 1;
    }
    cells.truncate(count);
    cells
}

fn layout(machine: &str) -> (Vec<E>, serde_json::Value) {
    let bays = bays(machine);
    let centres = centres(bays.len());
    let mut items = Vec::new();
    let mut slots = Vec::new();
    let mut groups: Vec<(&str, [f64; 2], [f64; 2])> = Vec::new();
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for (i, (mut bay, centre)) in bays.into_iter().zip(centres).enumerate() {
        let lo = [centre[0] - HALF, centre[1] - 7.0];
        let hi = [centre[0] + HALF, centre[1] + HALF];
        if groups.last().is_none_or(|g| g.0 != bay.section) { groups.push((bay.section, lo, hi)); }
        let group = groups.last_mut().unwrap();
        for axis in 0..2 {
            group.1[axis] = group.1[axis].min(lo[axis]);
            group.2[axis] = group.2[axis].max(hi[axis]);
            min[axis] = min[axis].min(lo[axis]);
            max[axis] = max[axis].max(hi[axis]);
        }
        let ns = format!("{NS}/exhibition/{}/{}", bay.section, bay.name);
        for e in &mut bay.items {
            if e.props().get("frame").is_none_or(|frame| frame == "world") {
                e.px += centre[0];
                e.py += centre[1];
            }
            let label_distance = if bay.section == "styles" { "80" } else { "35" };
            *e = e.clone().with_prop("label_max_distance", label_distance);
        }
        let code = format!("B{:03}", i + 1);
        items.push(E::text(&ns, "bay-code", &[[centre[0] as f32, centre[1] as f32 - 5.0, 0.2]], &format!("{code} / {}", bay.name))
            .with_width(-16.0).with_color(color::WHITE).with_prop("label_max_distance", "150"));
        slots.push(serde_json::json!({"id": code, "name": bay.name, "section": bay.section, "namespace": ns,
            "bounds": {"min": [lo[0], lo[1], 0.0], "max": [hi[0], hi[1], 5.0]},
            "focus": [centre[0], centre[1], 0.7], "distance": 16, "pitch": 62, "bearing": 180,
            "helpers": bay.helpers, "action": bay.action, "entities": bay.items.len()}));
        items.extend(bay.items);
    }
    let sections: Vec<_> = groups.into_iter().map(|(name, lo, hi)| serde_json::json!({"id": name, "name": name,
        "bounds": {"min": [lo[0], lo[1], 0.0], "max": [hi[0], hi[1], 5.0]},
        "focus": [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1]), 0.5],
        "distance": ((hi[0] - lo[0]) * 0.75).max((hi[1] - lo[1]) * 1.4).max(20.0), "pitch": 82, "bearing": 180})).collect();
    let manifest = serde_json::json!({"scene": "exhibition", "spacing_m": PITCH, "machine": machine,
        "bounds": {"min": [min[0], min[1], 0.0], "max": [max[0], max[1], 5.0]},
        "overview": {"focus": [0.5 * (max[0] + min[0]), 0.5 * (min[1] + max[1]), 0.0],
            "distance": ((max[0] - min[0]) * 0.75).max((max[1] - min[1]) * 1.4), "pitch": 82, "bearing": 180},
        "sections": sections, "slots": slots});
    (items, manifest)
}

pub(super) fn scene(machine: &str) -> Vec<E> { layout(machine).0 }
pub(super) fn manifest(machine: &str) -> serde_json::Value { layout(machine).1 }

pub(super) fn action(name: &str, machine: &str) -> Result<Vec<E>> {
    let manifest = manifest(machine);
    let slot = manifest["slots"].as_array().unwrap().iter().find(|slot| slot["action"].as_str() == Some(name))
        .ok_or_else(|| format!("unknown exhibition action {name}"))?;
    let kind = match name { "handle" => interact_kind::HANDLE, "pick-point" => interact_kind::PICK_POINT,
        "pick-pose" => interact_kind::PICK_POSE, "step" => interact_kind::STEP, _ => unreachable!() };
    let ns = slot["namespace"].as_str().unwrap();
    let position = std::array::from_fn(|i| slot["focus"][i].as_f64().unwrap());
    let mut items: Vec<_> = manifest["slots"].as_array().unwrap().iter().filter(|slot| !slot["action"].is_null())
        .map(|slot| E::delete(slot["namespace"].as_str().unwrap(), "request")).collect();
    let tint = match kind { interact_kind::STEP => color::MAGENTA, interact_kind::HANDLE => color::ORANGE, _ => color::GREEN };
    items.push(E::interact(ns, "request", kind).at_yaw(position, 0.0).with_color(tint)
        .with_prop("panel_finish", if kind == interact_kind::STEP { "white" } else { "black" })
        .with_prop("text", name).with_prop("controls", "move,rotate").with_prop("menu", "Inspect,Reset"));
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use gearbox_api::draw_kind;

    #[test]
    fn exhibition_covers_kinds_styles_shapes_panels_and_helper_families() {
        let items = scene("fixture_robot");
        for e in &items { e.validate().unwrap_or_else(|err| panic!("{}/{}: {err}", e.ns(), e.id())); }
        for kind in 1..=draw_kind::LAST { assert!(items.iter().any(|e| e.kind == kind), "kind {kind}"); }
        for look in style::PLAIN..=style::FIELD { assert!(items.iter().any(|e| e.style == look), "style {look}"); }
        for shape in 0..=shape::SECTOR { assert!(items.iter().any(|e| e.kind == draw_kind::SHAPES && e.sub == shape), "shape {shape}"); }
        let panels: Vec<_> = items.iter().filter(|e| e.kind == draw_kind::PANEL && e.ns().contains("/panels/")).collect();
        assert_eq!(panels.len(), 6);
        for panel in 0..=panel_kind::LEGEND { assert!(panels.iter().any(|e| e.sub == panel)); }
        for finish in ["color", "white", "black"] {
            assert!(panels.iter().any(|e| e.props().get("panel_finish").as_deref() == Some(finish)));
        }
        let m = manifest("fixture_robot");
        let helpers: BTreeSet<_> = m["slots"].as_array().unwrap().iter().flat_map(|s| s["helpers"].as_array().unwrap()).map(|h| h.as_str().unwrap()).collect();
        for helper in ["frame", "pose_array", "waypoints", "covariance", "ruler", "detections_3d", "detections_2d", "cloud", "scan", "range_cone",
            "camera_frustum", "particles", "field", "headland", "swaths", "turns", "ab_line", "coverage", "implement", "zone", "wrench", "contact",
            "graph", "ghost_trail", "panel", "gauge", "chip", "table", "legend", "badge", "compass", "scale_bar", "callout", "angle", "voxels",
            "odometry", "band", "rollouts", "path_with_markers", "tf_tree", "terrain_marks", "route_stations", "destination", "clearance_box", "field_volume"] {
            assert!(helpers.contains(helper), "missing helper {helper}");
        }
        assert_eq!(m["sections"].as_array().unwrap().len(), 12);
    }

    #[test]
    fn style_samples_do_not_repeat_styles_already_shown_by_real_helpers() {
        let bays = bays("fixture_robot");
        let represented: BTreeSet<_> = bays.iter().filter(|bay| bay.section != "styles")
            .flat_map(|bay| bay.items.iter().map(|e| e.style)).collect();
        for bay in bays.iter().filter(|bay| bay.section == "styles") {
            assert!(bay.items.iter().all(|e| !represented.contains(&e.style)), "{}", bay.name);
        }
    }

    #[test]
    fn standalone_label_survives_the_portrait_section_view() {
        let (items, manifest) = layout("fixture_robot");
        let camera = super::super::focus::camera(&manifest, "styles", 600.0, 1200.0, 58.0).unwrap();
        let label = items.iter().find(|e| e.style == style::LABEL && e.kind == gearbox_api::draw_kind::TEXT).unwrap();
        let range: f64 = label.props().get("label_max_distance").unwrap().parse().unwrap();
        let distance = camera["distance"].as_f64().unwrap();
        assert!(range > distance + 5.0);
    }

    #[test]
    fn ruler_and_scale_bar_share_one_visual_bay() {
        let bays = bays("fixture_robot");
        let measures: Vec<_> = bays.iter().filter(|bay| bay.helpers.iter().any(|h| ["ruler", "scale_bar"].contains(h))).collect();
        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].helpers, ["ruler", "scale_bar"]);
        assert_eq!(measures[0].items.len(), 1);
        assert_eq!(measures[0].items[0].style, style::RULER);
    }

    #[test]
    fn exhibition_keeps_all_bays_near_the_live_machine() {
        let slots = manifest("fixture_robot")["slots"].as_array().unwrap().clone();
        assert_eq!(slots[0]["focus"], serde_json::json!([0.0, 0.0, 0.7]));
        for slot in slots {
            for axis in 0..2 { assert!(slot["focus"][axis].as_f64().unwrap().abs() <= 90.0); }
        }
    }

    #[test]
    fn bays_are_disjoint_and_geometry_centres_stay_inside_their_bounds() {
        let m = manifest("fixture_robot");
        let slots = m["slots"].as_array().unwrap();
        let mut ids = BTreeSet::new();
        for (i, a) in slots.iter().enumerate() {
            assert!(ids.insert(a["id"].as_str().unwrap()));
            for b in &slots[i + 1..] {
                let separated = (0..2).any(|axis| a["bounds"]["max"][axis].as_f64().unwrap() < b["bounds"]["min"][axis].as_f64().unwrap()
                    || b["bounds"]["max"][axis].as_f64().unwrap() < a["bounds"]["min"][axis].as_f64().unwrap());
                assert!(separated, "overlapping {} {}", a["id"], b["id"]);
            }
        }
        for bay in bays("fixture_robot") {
            for e in bay.items {
                let pose = Pose { position: [e.px, e.py, e.pz], rotation: [e.qw, e.qx, e.qy, e.qz] };
                let instances = e.instance_list();
                for point in e.point_list().into_iter().chain(instances.iter().map(|i| i.position)) {
                    let world = pose.apply(point);
                    assert!(world[0].abs() <= HALF as f32 && world[1].abs() <= HALF as f32,
                        "{}/{} point {:?} exceeds bay", bay.section, bay.name, world);
                }
                if e.kind == draw_kind::GRID {
                    let props = e.props();
                    let cell = props.get_f64("cell").unwrap() as f32;
                    let corner = pose.apply([cell * props.get_f64("cols").unwrap() as f32, cell * props.get_f64("rows").unwrap() as f32, 0.0]);
                    assert!(corner[0].abs() <= HALF as f32 && corner[1].abs() <= HALF as f32);
                }
                if e.kind == draw_kind::SHAPES {
                    for instance in instances {
                        let local = Pose { position: instance.position.map(|v| v as f64), rotation: instance.rotation.map(|v| v as f64) };
                        let half = instance.size.map(|v| v.abs() * 0.5);
                        let (min, max) = match e.sub {
                            shape::FRUSTUM => {
                                let [h, v, reach] = instance.size;
                                let y = reach * (h * 0.5).tan();
                                let z = reach * (v * 0.5).tan();
                                ([0.0, -y, -z], [reach, y, z])
                            }
                            shape::SECTOR => {
                                let radius = instance.size[0].abs();
                                ([-radius, -radius, 0.0], [radius, radius, 0.0])
                            }
                            _ => (half.map(|v| -v), half),
                        };
                        for k in 0..8 {
                            let corner = std::array::from_fn(|axis| if k & (1 << axis) == 0 { min[axis] } else { max[axis] });
                            let point = pose.apply(local.apply(corner));
                            assert!(point[0].abs() <= HALF as f32 && point[1].abs() <= HALF as f32,
                                "{}/{} shape corner {:?} exceeds bay", bay.section, bay.name, point);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn normal_bays_keep_energy_defaults_and_controls_are_independent() {
        let items = scene("fixture_robot");
        for e in items.iter().filter(|e| e.ns().contains("/styles/") && e.id() != "bay-code" && e.id() != "bay-name") {
            assert_eq!((e.pulse_hz, e.flow_speed), (0.0, 0.0));
        }
        let control = |name: &str| items.iter().find(|e| e.ns().ends_with(&format!("controls/{name}/field")) && e.kind == draw_kind::SHAPES).unwrap();
        assert_eq!((control("aura-off-energy-on").glow, control("aura-off-energy-on").pulse_hz), (-1.0, 0.5));
        assert_eq!(control("motion-off").pulse_hz, -1.0);
        assert_eq!(control("alpha-zero").color >> 24, 0);
        assert_eq!(control("explicit-magenta").color, color::MAGENTA);
        let legend = items.iter().find(|e| e.kind == draw_kind::PANEL && e.sub == panel_kind::LEGEND).unwrap();
        assert_eq!(legend.props().get("world_anchor").as_deref(), Some("true"));
        assert_eq!(legend.props().get("label_max_distance").as_deref(), Some("35"));
        let active: Vec<_> = items.iter().filter(|e| e.kind == draw_kind::INTERACT).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].sub, interact_kind::HANDLE);
        for mode in ["handle", "pick-point", "pick-pose", "step"] {
            let activated = action(mode, "fixture_robot").unwrap();
            assert_eq!(activated.iter().filter(|e| e.op == draw_op::DELETE).count(), 4);
            assert_eq!(activated.iter().filter(|e| e.op == draw_op::UPSERT).count(), 1);
            for e in activated { assert!(e.ns().starts_with("vfx-gallery/exhibition/interactions/")); }
        }
    }
}
