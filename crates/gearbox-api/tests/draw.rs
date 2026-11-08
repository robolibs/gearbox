use std::time::Duration;

use gearbox_api::draw::DrawStore;
use gearbox_api::fake::FakeHost;
use gearbox_api::wire::draw::{DrawFrame, ns_within, parse_color, rgba, unpack_rgba};
use gearbox_api::wire::draw_geom::{
    colormap_rgba, covariance_axes, covariance_ellipse, in_ring, quantize, ring_area, rotate, triangulate, value_range,
};
use gearbox_api::wire::draw_widgets::{self as w, Place, Pose};
use gearbox_api::{
    Client, DRAW_CHUNK_BYTES, DrawEntity, GridCells, IdentitySource, Instance, MAX_PAYLOAD_BYTES, clear_scope, colormap, draw_color,
    draw_flag, draw_kind, draw_op, draw_style, line_mode, pack, panel_kind, shape_kind, unpack,
};

fn ring(n: usize, radius: f32, z: f32) -> Vec<[f32; 3]> {
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            [radius * a.cos(), radius * a.sin(), z]
        })
        .collect()
}

fn rollouts(count: usize, len: usize) -> DrawEntity {
    let strips: Vec<Vec<[f32; 3]>> =
        (0..count).map(|k| (0..len).map(|i| [i as f32 * 0.1, k as f32 * 0.01, 0.0]).collect()).collect();
    let colors: Vec<u32> = (0..count * len).map(|i| rgba((i % 255) as u8, 80, 200, 40)).collect();
    DrawEntity::strips("ondrive/robot/rollouts", "dwa", &strips).with_colors(&colors).with_width(-1.0).in_frame("robot/odom")
}

#[test]
fn a_draw_entity_survives_the_wire() {
    let entity = DrawEntity::shapes(
        "lab/obstacles",
        "pred",
        shape_kind::ELLIPSE,
        &[Instance::at([1.0, 2.0, 0.0], [0.8, 0.4, 0.0]).yawed(0.5), Instance::at([3.0, 0.0, 0.1], [1.2, 1.2, 0.0])],
    )
    .with_colors(&[rgba(255, 0, 0, 80), rgba(0, 0, 255, 80)])
    .with_fill(draw_flag::FILL_BOTH)
    .with_flags(draw_flag::LABELS)
    .with_labels(&["near", "far"])
    .with_lifetime(1.5)
    .with_pose([1.0, -2.0, 0.5], [0.0, 0.0, 0.0, 1.0])
    .with_style(draw_style::SHIELD)
    .with_glow(1.5)
    .with_pulse(0.7)
    .with_flow(2.0, 0.8)
    .with_beam(3.0)
    .with_fade(0.5)
    .with_urgency(0.25)
    .in_frame("husky/base_link");
    let env = pack(&entity);
    assert!(env.wire().len() < MAX_PAYLOAD_BYTES);
    let back: DrawEntity = unpack(env.type_hash(), env.wire()).expect("decode");
    assert_eq!(back, entity);
    assert_eq!(back.ns(), "lab/obstacles");
    assert_eq!(back.frame(), DrawFrame::Link("husky".into(), "base_link".into()));
    assert_eq!(back.instance_list()[0].position, [1.0, 2.0, 0.0]);
    assert_eq!(back.label_list(), ["near", "far"]);
    assert_eq!((back.style, back.glow, back.pulse_hz, back.flow_speed, back.dash), (draw_style::SHIELD, 1.5, 0.7, 2.0, 0.8));
    assert_eq!((back.beam_height, back.fade, back.urgency), (3.0, 0.5, 0.25));
    assert!(back.validate().is_ok());
}

#[test]
fn frames_colours_and_paths_parse() {
    assert_eq!(DrawFrame::parse(""), DrawFrame::World);
    assert_eq!(DrawFrame::parse("world"), DrawFrame::World);
    assert_eq!(DrawFrame::parse("husky/odom"), DrawFrame::Odom("husky".into()));
    assert_eq!(DrawFrame::parse("husky"), DrawFrame::Link("husky".into(), String::new()));
    assert_eq!(parse_color("#ff8000"), Some(rgba(255, 128, 0, 255)));
    assert_eq!(parse_color("#ff800040"), Some(rgba(255, 128, 0, 64)));
    assert_eq!(parse_color("#fff"), Some(rgba(255, 255, 255, 255)));
    assert!(parse_color("nope").is_none());
    assert!(ns_within("ondrive/robot/rollouts", "ondrive/robot"));
    assert!(ns_within("ondrive/robot", "ondrive/robot"));
    assert!(!ns_within("ondrive/robot2", "ondrive/robot"));
    assert!(ns_within("anything", ""));
}

#[test]
fn invalid_entities_are_refused() {
    let mut store = DrawStore::default();
    let no_id = DrawEntity::lines("a", "", &[[0.0; 3], [1.0; 3]]);
    assert!(store.apply(no_id, 0.0).is_err());
    let bad_colors = DrawEntity::lines("a", "b", &[[0.0; 3], [1.0; 3], [2.0; 3]]).with_colors(&[1, 2]);
    assert!(store.apply(bad_colors, 0.0).is_err());
    let bad_offsets = DrawEntity::lines("a", "b", &[[0.0; 3], [1.0; 3]]).with_offsets(&[0, 5]);
    assert!(store.apply(bad_offsets, 0.0).is_err());
    assert!(store.is_empty());
}

#[test]
fn chunks_fit_a_message_and_assemble_into_the_entity() {
    let entity = rollouts(300, 40);
    let chunks = entity.chunks(DRAW_CHUNK_BYTES);
    assert!(chunks.len() > 4, "{} chunks", chunks.len());
    for chunk in &chunks {
        assert!(pack(chunk).wire().len() <= MAX_PAYLOAD_BYTES, "{} bytes", pack(chunk).wire().len());
        assert_eq!(chunk.op, draw_op::APPEND);
        assert!(chunk.validate().is_ok());
    }
    assert_eq!(chunks.iter().filter(|c| c.last == 1).count(), 1);

    let mut store = DrawStore::default();
    store.apply(DrawEntity::lines("ondrive/robot/rollouts", "dwa", &[[0.0; 3], [1.0; 3]]), 0.0).unwrap();
    for (i, chunk) in chunks.iter().enumerate() {
        store.apply(chunk.clone(), 0.0).unwrap();
        let live = &store.get("ondrive/robot/rollouts", "dwa").unwrap().entity;
        if i + 1 < chunks.len() {
            assert_eq!(live.point_count(), 2, "the old entity stays until the last chunk");
        }
    }
    let live = &store.get("ondrive/robot/rollouts", "dwa").unwrap().entity;
    assert_eq!(live.point_count(), entity.point_count());
    assert_eq!(live.points, entity.points);
    assert_eq!(live.colors, entity.colors);
    assert_eq!(live.strip_ranges(), entity.strip_ranges());
    assert_eq!(live.op, draw_op::UPSERT);
}

#[test]
fn a_strip_longer_than_a_chunk_stays_connected() {
    let long = ring(5000, 10.0, 0.0);
    let entity = DrawEntity::lines("lab", "path", &long).with_sub(line_mode::LOOP).with_labels(&["path"]);
    let chunks = entity.chunks(DRAW_CHUNK_BYTES);
    assert!(chunks.len() > 2);
    let mut store = DrawStore::default();
    for chunk in chunks {
        store.apply(chunk, 0.0).unwrap();
    }
    let live = &store.get("lab", "path").unwrap().entity;
    assert_eq!(live.sub, line_mode::STRIP, "a split loop is closed and sent as strips");
    let strips = live.strip_ranges();
    for pair in strips.windows(2) {
        assert_eq!(live.point_at(pair[0].end - 1), live.point_at(pair[1].start), "pieces share their joint");
    }
    assert_eq!(live.point_at(0), live.point_at(live.point_count() - 1), "the loop closes");
    assert_eq!(live.label_list().len(), strips.len());
    assert_eq!(live.label_list()[0], "path");
}

#[test]
fn instanced_entities_chunk_by_instance() {
    let instances: Vec<Instance> = (0..1000).map(|i| Instance::at([i as f32, 0.0, 0.0], [0.5, 0.5, 0.5])).collect();
    let colors: Vec<u32> = (0..1000).map(|i| rgba(i as u8, 0, 0, 255)).collect();
    let entity = DrawEntity::shapes("lab", "boxes", shape_kind::BOX, &instances).with_colors(&colors);
    let chunks = entity.chunks(DRAW_CHUNK_BYTES);
    assert!(chunks.len() > 2);
    let mut store = DrawStore::default();
    for chunk in chunks {
        store.apply(chunk, 0.0).unwrap();
    }
    let live = &store.get("lab", "boxes").unwrap().entity;
    assert_eq!(live.instance_list(), instances);
    assert_eq!(live.colors, colors);
}

#[test]
fn out_of_order_chunks_are_refused_and_a_new_seq_restarts() {
    let chunks = rollouts(200, 40).chunks(DRAW_CHUNK_BYTES);
    let mut store = DrawStore::default();
    store.apply(chunks[0].clone(), 0.0).unwrap();
    assert!(store.apply(chunks[2].clone(), 0.0).is_err());
    assert!(store.get("ondrive/robot/rollouts", "dwa").is_none());
    let again = rollouts(200, 40).chunks(DRAW_CHUNK_BYTES);
    for chunk in again {
        store.apply(chunk, 0.0).unwrap();
    }
    assert!(store.get("ondrive/robot/rollouts", "dwa").is_some());
}

#[test]
fn clear_removes_a_path_and_everything_below_it() {
    let mut store = DrawStore::default();
    for (ns, id) in [("ondrive/robot", "path"), ("ondrive/robot/rollouts", "dwa"), ("ondrive/robot2", "path"), ("lab", "goal")] {
        store.apply(DrawEntity::points(ns, id, &[[0.0; 3]]), 0.0).unwrap();
    }
    assert_eq!(store.apply(DrawEntity::clear("ondrive/robot"), 0.0).unwrap(), 2);
    let left: Vec<String> = store.iter().map(|((ns, id), _)| format!("{ns}:{id}")).collect();
    assert_eq!(left, ["lab:goal", "ondrive/robot2:path"]);
    assert_eq!(store.apply(DrawEntity::delete("lab", "goal"), 0.0).unwrap(), 1);
    assert_eq!(store.apply(DrawEntity::clear(""), 0.0).unwrap(), 1);
    assert!(store.is_empty());
}

#[test]
fn lifetimes_run_in_the_clock_given_and_a_resend_renews_them() {
    let mut store = DrawStore::default();
    let short = DrawEntity::points("lab", "blink", &[[0.0; 3]]).with_lifetime(1.0);
    store.apply(short.clone(), 10.0).unwrap();
    store.apply(DrawEntity::points("lab", "keep", &[[0.0; 3]]), 10.0).unwrap();
    store.expire(10.5);
    store.apply(short, 10.5).unwrap();
    store.expire(11.2);
    assert!(store.get("lab", "blink").is_some(), "resent at 10.5, lives to 11.5");
    store.expire(11.6);
    assert!(store.get("lab", "blink").is_none());
    assert!(store.get("lab", "keep").is_some());
}

#[test]
fn extend_grows_a_trace_and_caps_it() {
    let mut store = DrawStore::default();
    for i in 0..10 {
        let step = DrawEntity { op: draw_op::EXTEND, ..DrawEntity::lines("lab", "trace", &[[i as f32, 0.0, 0.0]]) }
            .with_prop("max_points", "6");
        store.apply(step, 0.0).unwrap();
    }
    let live = &store.get("lab", "trace").unwrap().entity;
    assert_eq!(live.point_count(), 6);
    assert_eq!(live.point_at(0), [4.0, 0.0, 0.0]);
    assert_eq!(live.strip_ranges().len(), 1, "extending continues the strip");
    let revision = store.get("lab", "trace").unwrap().revision;
    let more = DrawEntity { op: draw_op::EXTEND, ..DrawEntity::lines("lab", "trace", &[[20.0, 0.0, 0.0]]) };
    store.apply(more, 0.0).unwrap();
    assert!(store.get("lab", "trace").unwrap().revision > revision);
}

#[test]
fn draws_have_a_json_form() {
    use gearbox_api::wire::json::{env_from_json, env_to_json};
    let request = serde_json::json!({
        "kind": "lines", "ns": "lab", "id": "p", "points": [[0, 0], [1, 0, 0.5]], "color": "#ff8000",
        "width": -3, "frame": "husky/odom", "labels": ["a"], "sub": "loop",
    });
    let env = env_from_json("draw", &request).expect("draw from json");
    let e: DrawEntity = unpack(env.type_hash(), env.wire()).expect("decode");
    assert_eq!(e.kind, draw_kind::LINES);
    assert_eq!(e.sub, line_mode::LOOP);
    assert_eq!(e.point_list(), [[0.0, 0.0, 0.0], [1.0, 0.0, 0.5]]);
    assert_eq!(e.color, rgba(255, 128, 0, 255));
    assert_eq!(e.frame(), DrawFrame::Odom("husky".into()));
    assert_eq!((e.width, e.last, e.qw), (-3.0, 1, 1.0));
    assert!(e.validate().is_ok());
    let back = env_to_json(&env).expect("to json");
    assert_eq!(back["kind"], "lines");
    assert_eq!(back["props"]["ns"], "lab");
    assert_eq!(back["color"], "#ff8000ff");
}

#[test]
fn new_header_fields_and_sections_survive_the_wire() {
    let e = DrawEntity::polygons("lab/zones", "p", &[vec![square(0.0, 4.0), square(1.0, 2.0)], vec![square(10.0, 3.0)]])
        .with_values(&[0.25, 0.75])
        .with_colormap(colormap::RDYLGN, 1.0, 0.0)
        .with_style(draw_style::HEAT)
        .with_flags(draw_flag::ARROW_TIP)
        .with_prop("z_offset", "0.1");
    let mut e = e;
    e.qpoints = vec![1, -2, 3];
    e.points.clear();
    e.ranges = vec![];
    e.cells = vec![7, 8];
    e.quant = 0.02;
    let back: DrawEntity = unpack(pack(&e).type_hash(), pack(&e).wire()).expect("decode");
    assert_eq!(back, e);
    assert_eq!((back.vmin, back.vmax, back.quant, back.colormap), (1.0, 0.0, 0.02, colormap::RDYLGN));
    assert_eq!(back.rings, vec![2, 1]);
    assert_eq!(back.qpoints, vec![1, -2, 3]);
    assert_eq!(back.cells, vec![7, 8]);
    let scan = DrawEntity::scan("lab", "scan", &[1.0, f32::INFINITY, 2.5], -0.5, 0.5);
    let back: DrawEntity = unpack(pack(&scan).type_hash(), pack(&scan).wire()).expect("decode");
    assert_eq!(back.ranges, vec![1000, gearbox_api::wire::draw::NO_RETURN, 2500]);
    assert!(back.validate().is_ok());
}

fn square(at: f32, side: f32) -> Vec<[f32; 3]> {
    vec![[at, at, 0.0], [at + side, at, 0.0], [at + side, at + side, 0.0], [at, at + side, 0.0]]
}

fn tri_area(p: &[[f32; 2]], t: &[u32; 3]) -> f32 {
    let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
    0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs()
}

#[test]
fn triangulation_fills_a_polygon_around_its_holes() {
    let outer = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
    let hole_a = [[2.0, 2.0], [4.0, 2.0], [4.0, 4.0], [2.0, 4.0]];
    let hole_b = [[6.0, 6.0], [8.0, 6.0], [7.0, 8.0]];
    let tris = triangulate(&[&outer, &hole_a, &hole_b]);
    let all: Vec<[f32; 2]> = outer.iter().chain(&hole_a).chain(&hole_b).copied().collect();
    let area: f32 = tris.iter().map(|t| tri_area(&all, t)).sum();
    assert!((area - (100.0 - 4.0 - 2.0)).abs() < 1e-3, "covered {area}");
    for t in &tris {
        let c = [0, 1].map(|k| (all[t[0] as usize][k] + all[t[1] as usize][k] + all[t[2] as usize][k]) / 3.0);
        assert!(!in_ring(&hole_a, c) && !in_ring(&hole_b, c), "a triangle inside a hole at {c:?}");
    }
    let concave = [[0.0, 0.0], [6.0, 0.0], [6.0, 6.0], [3.0, 2.0], [0.0, 6.0]];
    let tris = triangulate(&[&concave]);
    assert_eq!(tris.len(), 3);
    let area: f32 = tris.iter().map(|t| tri_area(&concave, t)).sum();
    assert!((area - ring_area(&concave).abs()).abs() < 1e-3);
    assert!(triangulate(&[&[[0.0, 0.0], [1.0, 1.0]]]).is_empty());
}

#[test]
fn colormaps_span_their_range_and_palettes_follow_rviz() {
    assert_eq!(colormap_rgba(colormap::RDYLGN, 0.0, 0.0, 1.0), unpack_rgba(draw_color::RED));
    assert_eq!(colormap_rgba(colormap::RDYLGN, 1.0, 0.0, 1.0), unpack_rgba(draw_color::GREEN));
    assert_eq!(colormap_rgba(colormap::RDYLGN, 0.5, 0.0, 1.0), unpack_rgba(draw_color::YELLOW));
    assert_eq!(colormap_rgba(colormap::RDYLGN, 0.0, 1.0, 0.0), unpack_rgba(draw_color::GREEN), "vmin above vmax runs it backwards");
    let viridis_low = colormap_rgba(colormap::VIRIDIS, 0.0, 0.0, 1.0);
    let viridis_high = colormap_rgba(colormap::VIRIDIS, 1.0, 0.0, 1.0);
    assert!(viridis_low[2] > viridis_low[1] && viridis_high[1] > viridis_high[2], "{viridis_low:?} {viridis_high:?}");
    let turbo_mid = colormap_rgba(colormap::TURBO, 0.5, 0.0, 1.0);
    assert!(turbo_mid[1] > 200, "turbo is green-yellow in the middle: {turbo_mid:?}");
    assert_eq!(colormap_rgba(colormap::COSTMAP, 0.0, 0.0, 0.0)[3], 0, "free cells are clear");
    assert_eq!(colormap_rgba(colormap::COSTMAP, 99.0, 0.0, 0.0), [0, 255, 255, 255], "inscribed is cyan");
    assert_eq!(colormap_rgba(colormap::COSTMAP, 100.0, 0.0, 0.0), [255, 0, 255, 255], "lethal is purple");
    assert_eq!(colormap_rgba(colormap::COSTMAP, -1.0, 0.0, 0.0), [0x70, 0x89, 0x86, 255]);
    assert_eq!(colormap_rgba(colormap::MAP, 0.0, 0.0, 0.0), [255, 255, 255, 255]);
    assert_eq!(colormap_rgba(colormap::MAP, 100.0, 0.0, 0.0), [0, 0, 0, 255]);
    assert_eq!(colormap_rgba(colormap::OKABE, 9.0, 0.0, 0.0), unpack_rgba(draw_color::CYAN));
    assert_eq!(value_range([2.0, 5.0, 3.0], 0.0, 0.0), (2.0, 5.0), "equal ends take the data's range");
    assert_eq!(colormap::parse("cost"), Some(colormap::RDYLGN));
    assert_eq!(parse_color("plan"), Some(draw_color::CYAN));
}

#[test]
fn covariance_axes_are_the_square_roots_of_the_eigenvalues() {
    let (a, b, angle) = covariance_ellipse(4.0, 0.0, 1.0);
    assert!((a - 2.0).abs() < 1e-9 && (b - 1.0).abs() < 1e-9 && angle.abs() < 1e-9);
    let (a, b, angle) = covariance_ellipse(1.0, 0.0, 9.0);
    assert!((a - 3.0).abs() < 1e-9 && (b - 1.0).abs() < 1e-9);
    assert!((angle.abs() - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "the major axis along y");
    let (a, b, angle) = covariance_ellipse(2.0, 1.0, 2.0);
    assert!((a - 3f64.sqrt()).abs() < 1e-9 && (b - 1.0).abs() < 1e-9);
    assert!((angle - std::f64::consts::FRAC_PI_4).abs() < 1e-9, "tilted 45°");
    let (sd, q) = covariance_axes([[1.0, 0.0, 0.0], [0.0, 9.0, 0.0], [0.0, 0.0, 4.0]]);
    assert!((sd[0] - 3.0).abs() < 1e-9 && (sd[1] - 2.0).abs() < 1e-9 && (sd[2] - 1.0).abs() < 1e-9);
    let x = rotate(q, [1.0, 0.0, 0.0]);
    assert!(x[1].abs() > 0.999, "the largest axis turned onto y: {x:?}");
    let c = [[2.0, 0.5, 0.1], [0.5, 1.0, 0.2], [0.1, 0.2, 0.5]];
    let (sd, q) = covariance_axes(c);
    for k in 0..3 {
        let mut axis = [0.0; 3];
        axis[k] = 1.0;
        let v = rotate(q, axis);
        let cv = [0, 1, 2].map(|r| (0..3).map(|j| c[r][j] * v[j]).sum::<f64>());
        for r in 0..3 {
            assert!((cv[r] - sd[k] * sd[k] * v[r]).abs() < 1e-6, "axis {k} is an eigenvector");
        }
    }
}

#[test]
fn quantised_points_and_scans_expand_back() {
    let pts = [[1.234, -5.678, 0.5], [300.0, -300.0, 0.0]];
    let e = DrawEntity::new(draw_kind::POINTS, "lab", "q").with_quantized_points(&pts, 0.01);
    assert_eq!(e.qpoints, vec![123, -568, 50, 30000, -30000, 0]);
    assert_eq!(e.point_count(), 2);
    let back = e.expanded();
    assert!(back.qpoints.is_empty());
    for (a, b) in back.point_list().iter().zip(pts) {
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() <= 0.005 + 1e-4, "{a:?} {b:?}");
        }
    }
    assert_eq!(quantize(&[[400.0, 0.0, 0.0]], 0.01)[0], i16::MAX, "clamped at ±327 m for 1 cm");
    let bytes = DrawEntity::points("lab", "v", &[[0.0; 3]; 3]).with_byte_values(&[0.0, 5.0, 10.0], 0.0, 10.0);
    assert_eq!(bytes.cells, vec![0, 128, 255]);
    let vals = bytes.expanded().values;
    assert!((vals[1] - 5.0).abs() < 0.03 && vals[2] == 10.0);
    let scan = DrawEntity::scan("lab", "s", &[1.0, f32::NAN, 2.0], 0.0, std::f32::consts::FRAC_PI_2);
    let pts = scan.expanded().point_list();
    assert_eq!(pts.len(), 2, "no-return beams are left out");
    assert!((pts[0][0] - 1.0).abs() < 1e-6 && (pts[1][0] + 2.0).abs() < 1e-3 && pts[1][1].abs() < 1e-3);
    assert!(gearbox_api::wire::draw_geom::dequantize(&[100], 0.01)[0] == 1.0);
}

#[test]
fn update_changes_the_header_and_keeps_the_points() {
    let mut store = DrawStore::default();
    let cloud: Vec<[f32; 3]> = (0..500).map(|i| [i as f32, 0.0, 0.0]).collect();
    store.apply(DrawEntity::points("lab", "cloud", &cloud).with_color(rgba(1, 2, 3, 255)).with_width(0.2), 0.0).unwrap();
    let moved = DrawEntity::update("lab", "cloud").at_yaw([5.0, 6.0, 0.0], 0.5).with_color(rgba(9, 9, 9, 255)).updating(&["pose"]);
    assert!(pack(&moved).wire().len() < 600, "a header-only message");
    store.apply(moved, 1.0).unwrap();
    let live = &store.get("lab", "cloud").unwrap().entity;
    assert_eq!((live.px, live.py), (5.0, 6.0));
    assert_eq!(live.color, rgba(1, 2, 3, 255), "only the pose was taken");
    assert_eq!(live.point_count(), 500);
    let restyle = DrawEntity::update("lab", "cloud").with_style(draw_style::SPLAT).with_width(-3.0).with_prop("note", "x").updating(&["style", "width"]);
    store.apply(restyle, 1.0).unwrap();
    let live = &store.get("lab", "cloud").unwrap().entity;
    assert_eq!((live.style, live.width), (draw_style::SPLAT, -3.0));
    assert_eq!(live.props().get("note").as_deref(), Some("x"));
    assert_eq!(live.id(), "cloud");
    assert!(store.apply(DrawEntity::update("lab", "missing"), 1.0).is_err());
}

#[test]
fn patch_replaces_a_range_and_grows_the_end() {
    let mut store = DrawStore::default();
    let pts: Vec<[f32; 3]> = (0..10).map(|i| [i as f32, 0.0, 0.0]).collect();
    store.apply(DrawEntity::points("lab", "p", &pts).with_color(rgba(1, 1, 1, 255)), 0.0).unwrap();
    let patch = DrawEntity::patch(draw_kind::POINTS, "lab", "p", 8)
        .with_points(&[[80.0, 0.0, 0.0], [90.0, 0.0, 0.0], [100.0, 0.0, 0.0]])
        .with_colors(&[rgba(9, 0, 0, 255), rgba(9, 0, 0, 255), rgba(9, 0, 0, 255)]);
    store.apply(patch, 0.0).unwrap();
    let live = &store.get("lab", "p").unwrap().entity;
    assert_eq!(live.point_count(), 11);
    assert_eq!(live.point_at(7), [7.0, 0.0, 0.0]);
    assert_eq!(live.point_at(10), [100.0, 0.0, 0.0]);
    assert_eq!(live.colors.len(), 11);
    assert_eq!(live.colors[7], 0, "the items not patched keep the entity's colour");
    assert_eq!(live.color, rgba(1, 1, 1, 255));
    assert_eq!(live.colors[9], rgba(9, 0, 0, 255));
    assert!(store.apply(DrawEntity::patch(draw_kind::POINTS, "lab", "p", 50).with_points(&[[0.0; 3]]), 0.0).is_err());
}

#[test]
fn grids_send_in_runs_and_patch_by_rectangles() {
    let (cols, rows) = (200, 150);
    let cells: Vec<u8> = (0..cols * rows).map(|i| (i % 101) as u8).collect();
    let grid = DrawEntity::grid("map", "costmap", cols, rows, 0.1, GridCells::U8(cells.clone())).with_colormap(colormap::COSTMAP, 0.0, 0.0);
    assert!(grid.validate().is_ok());
    let chunks = grid.chunks(DRAW_CHUNK_BYTES);
    assert!(chunks.len() >= 3, "{} chunks", chunks.len());
    let mut store = DrawStore::default();
    for c in &chunks {
        assert!(pack(c).wire().len() <= MAX_PAYLOAD_BYTES);
        store.apply(c.clone(), 0.0).unwrap();
    }
    let live = &store.get("map", "costmap").unwrap().entity;
    assert_eq!(live.cells, cells);
    assert_eq!(live.grid_dims(), (cols, rows, 0.1, 0.1));
    let tile = DrawEntity::grid_patch("map", "costmap", 10, 20, 3, 2, GridCells::U8(vec![100; 6]));
    store.apply(tile, 0.0).unwrap();
    let live = &store.get("map", "costmap").unwrap().entity;
    assert_eq!(live.cells[20 * cols + 10], 100);
    assert_eq!(live.cells[21 * cols + 12], 100);
    assert_eq!(live.cells[21 * cols + 13], cells[21 * cols + 13]);
    let off = DrawEntity::grid_patch("map", "costmap", 199, 0, 2, 1, GridCells::U8(vec![1, 2]));
    assert!(store.apply(off, 0.0).is_err(), "a patch off the grid is refused");
    let empty = DrawEntity::grid("map", "blank", 4, 4, 1.0, GridCells::F32(Vec::new())).with_prop("unknown", "-1");
    store.apply(empty, 0.0).unwrap();
    store.apply(DrawEntity::grid_patch("map", "blank", 1, 1, 1, 1, GridCells::F32(vec![0.5])), 0.0).unwrap();
    let live = &store.get("map", "blank").unwrap().entity;
    assert_eq!(live.cell_count(), 16);
    assert_eq!(live.cell_value(5), Some(0.5));
    assert_eq!(live.cell_value(0), None, "the rest is unknown");
    let bad = DrawEntity::grid("map", "bad", 4, 4, 1.0, GridCells::U8(vec![0; 5]));
    assert!(bad.validate().is_err());
}

#[test]
fn polygons_chunk_whole_and_a_huge_ring_goes_on_across_chunks() {
    let many: Vec<Vec<Vec<[f32; 3]>>> = (0..400).map(|k| vec![square(k as f32 * 3.0, 2.0), square(k as f32 * 3.0 + 0.5, 0.5)]).collect();
    let colors: Vec<u32> = (0..400).map(|k| rgba(k as u8, 0, 0, 255)).collect();
    let e = DrawEntity::polygons("lab", "many", &many).with_colors(&colors);
    let chunks = e.chunks(DRAW_CHUNK_BYTES);
    assert!(chunks.len() > 1);
    let mut store = DrawStore::default();
    for c in &chunks {
        assert!(c.validate().is_ok(), "{:?}", c.validate());
        store.apply(c.clone(), 0.0).unwrap();
    }
    let live = &store.get("lab", "many").unwrap().entity;
    assert_eq!(live.polygon_count(), 400);
    assert_eq!(live.colors, colors);
    assert_eq!(live.polygon_rings().iter().map(|p| p.len()).collect::<Vec<_>>(), vec![2; 400]);

    let ring: Vec<[f32; 3]> = (0..3000).map(|i| {
        let a = i as f32 / 3000.0 * std::f32::consts::TAU;
        [50.0 * a.cos(), 50.0 * a.sin(), 0.0]
    }).collect();
    let big = DrawEntity::polygons("lab", "big", &[vec![ring.clone(), square(-5.0, 10.0)], vec![square(100.0, 4.0)]]);
    let chunks = big.chunks(DRAW_CHUNK_BYTES);
    assert!(chunks.len() >= 3, "{} chunks", chunks.len());
    for c in &chunks {
        assert!(pack(c).wire().len() <= MAX_PAYLOAD_BYTES);
        store.apply(c.clone(), 0.0).unwrap();
    }
    let live = &store.get("lab", "big").unwrap().entity;
    let polys = live.polygon_rings();
    assert_eq!(polys.len(), 2);
    assert_eq!(polys[0].len(), 2, "the big outline and its hole");
    assert_eq!(polys[0][0].len(), 3000, "the outline is whole again");
    assert_eq!(live.points, big.points);
}

#[test]
fn panels_extend_as_sparklines() {
    let mut store = DrawStore::default();
    for i in 0..10 {
        let step = DrawEntity { op: draw_op::EXTEND, ..DrawEntity::panel("lab", "speed", panel_kind::SPARKLINE, &[i as f32]) }
            .with_prop("max_points", "4");
        store.apply(step, 0.0).unwrap();
    }
    let live = &store.get("lab", "speed").unwrap().entity;
    assert_eq!(live.values, vec![6.0, 7.0, 8.0, 9.0]);
}

#[test]
fn widgets_compose_entities_under_their_own_path() {
    let place = Place::frame("husky/odom");
    let parts = w::frame(&place, "lab/tf", "base", Pose::yawed(1.0, 2.0, 0.5, 0.3), Some([0.0; 3]), 0.5, None);
    assert_eq!(parts.iter().map(|e| e.ns()).collect::<Vec<_>>(), ["lab/tf/base"; 3]);
    assert!(parts.iter().all(|e| e.validate().is_ok()));
    assert_eq!(parts[0].kind, draw_kind::AXES);
    assert_eq!(parts[0].style, draw_style::ARROW_GLOW);
    let wp = w::waypoints(&place, "lab", "route", &[[0.0; 3], [5.0, 0.0, 0.0], [5.0, 5.0, 0.0]], 1);
    assert_eq!(wp.len(), 3);
    assert_eq!(wp[2].style, draw_style::BEACON);
    assert!(wp.iter().all(|e| e.flags & draw_flag::DRAPE != 0), "odom frames drape");
    let cov = w::covariance(&place, "lab", "cov", Pose::at(1.0, 1.0, 0.0), &[1.0, 0.2, 0.2, 0.5], &[1.0, 2.0], 0).unwrap();
    assert_eq!(cov[0].strip_ranges().len(), 2);
    assert!(w::covariance(&place, "lab", "cov", Pose::default(), &[1.0; 5], &[], 0).is_err());
    let mut six = [0.0; 36];
    (six[0], six[7], six[14], six[35]) = (0.2, 0.1, 0.05, 0.04);
    let cov6 = w::covariance(&place, "lab", "cov6", Pose::default(), &six, &[1.0], 0).unwrap();
    assert!(cov6.iter().any(|e| e.sub == shape_kind::SECTOR && e.style == draw_style::SWEEP));
    let dets = vec![w::Detection3d {
        pose: Pose::at(3.0, 0.0, 0.8),
        size: [4.0, 2.0, 1.6],
        class: "car".into(),
        score: 0.92,
        track: Some(17),
        velocity: Some([2.0, 0.0, 0.0]),
        trail: vec![[0.0; 3], [1.0, 0.0, 0.0]],
    }];
    let det = w::detections_3d(&place, "lab", "dets", &dets, &w::DetectionStyle::default());
    assert_eq!(det[1].label_list(), ["car 0.92 #17"]);
    assert_eq!(det[0].colors[0] & 0x00ff_ffff, draw_color::ORANGE & 0x00ff_ffff);
    assert!(det.iter().all(|e| e.validate().is_ok()));
}

#[test]
fn the_fake_host_keeps_draws_and_clears_them_with_markers() {
    let host = FakeHost::start("test-draw", &[]).expect("fake host");
    let c = Client::connect_id(host.endpoint_id, IdentitySource::Random, "drawer").expect("client");
    c.wait_ready(Duration::from_secs(10)).expect("host answers info");

    let status = c.draw(&rollouts(300, 40)).expect("draw");
    assert!(status.is_ok(), "{}", status.message());
    let arrow = DrawEntity::arrows("lab/goal", "g", &[([0.0; 3], [1.0, 0.0, 0.0])]).with_color(rgba(255, 200, 0, 255));
    assert!(c.draw(&arrow).expect("arrow").is_ok());
    let listed = c.draw_list("").expect("list");
    assert_eq!(listed.len(), 2);
    let rollout = listed.iter().find(|e| e.id() == "dwa").expect("rollouts listed");
    assert_eq!(rollout.props().get("points").as_deref(), Some("12000"));
    assert_eq!(rollout.kind, draw_kind::LINES);

    assert!(c.draw_clear("lab").expect("clear").is_ok());
    assert_eq!(c.draw_list("").expect("list").len(), 1);
    assert!(c.clear(clear_scope::MARKERS).expect("clear markers").is_ok());
    assert!(c.draw_list("").expect("list").is_empty());
}
