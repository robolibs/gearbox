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
fn non_finite_draw_headers_and_radii_never_mutate_retained_state() {
    let original = DrawEntity::lines("valid", "retained", &[[0.0; 3], [2.0, 0.0, 0.0]]).with_lifetime(5.0);
    let mut store = DrawStore::default();
    store.apply(original.clone(), 0.0).unwrap();
    let revision = store.revision();
    for op in [draw_op::UPSERT, draw_op::APPEND, draw_op::UPDATE, draw_op::PATCH, draw_op::EXTEND] {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for field in 0..8 {
                let mut e = original.clone();
                e.op = op;
                *[&mut e.px, &mut e.py, &mut e.pz, &mut e.qw, &mut e.qx, &mut e.qy, &mut e.qz, &mut e.lifetime_s][field] = invalid;
                assert!(store.apply(e, 2.0).is_err(), "op {op}, header {field}");
            }
            for field in 0..3 {
                let mut e = original.clone();
                e.op = op;
                *[&mut e.sx, &mut e.sy, &mut e.sz][field] = invalid as f32;
                assert!(store.apply(e, 2.0).is_err(), "op {op}, scale {field}");
            }
            let mut e = original.clone().with_radii(&[invalid as f32]);
            e.op = op;
            assert!(store.apply(e, 2.0).is_err(), "op {op}, radius");
            assert!(original.clone().with_lifetime(invalid).validate().is_err());
        }
    }
    let retained = store.get("valid", "retained").unwrap();
    assert_eq!(retained.entity, original);
    assert_eq!(retained.expires_s, Some(5.0));
    assert_eq!((store.revision(), retained.revision, retained.generation), (revision, revision, revision));
}

#[test]
fn finite_draw_sentinels_and_missing_grid_samples_remain_valid() {
    for value in [-2.0, 0.0, 2.0] {
        let e = DrawEntity::points("valid", "points", &[[0.0; 3]]).with_radii(&[value])
            .with_scale([value; 3]).with_width(value).with_glow(value)
            .with_pulse(value).with_flow(value, value).with_beam(value).with_fade(value);
        assert!(e.validate().is_ok());
    }
    assert_eq!(DrawEntity::update("valid", "points").with_lifetime(-1.0).lifetime_s, 0.0);
    let grid = DrawEntity::grid("valid", "unknown", 3, 1, 0.5, GridCells::F32(vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY]));
    assert!(grid.validate().is_ok());
    assert!(DrawEntity::scan("valid", "scan", &[f32::NAN, f32::INFINITY, -1.0, 3.0], 0.0, 0.1).validate().is_ok());
}

#[test]
fn panel_finishes_round_trip_without_recoloring_or_changing_samples() {
    for kind in 0..=panel_kind::LEGEND {
        for finish in ["color", "white", "black"] {
            let entity = DrawEntity::panel("glass", "sample", kind, &[0.25, 0.75])
                .with_color(rgba(207, 114, 168, 128)).with_colors(&[draw_color::GREEN, draw_color::MAGENTA])
                .with_colormap(colormap::VIRIDIS, -2.0, 4.0)
                .with_prop("panel_finish", finish).with_prop("panel_theme", "light");
            let env = pack(&entity);
            let back: DrawEntity = unpack(env.type_hash(), env.wire()).unwrap();
            assert_eq!(back, entity);
            back.validate().unwrap();
        }
    }
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
fn blur_roundtrips_and_updates_without_changing_geometry_or_light() {
    let original = DrawEntity::lines("glass", "trail", &[[0.0; 3], [1.0, 0.0, 0.0]])
        .with_blur(0.6).with_glow(0.4).with_color(draw_color::ORANGE).with_width(0.03);
    let encoded = pack(&original);
    let copied: DrawEntity = unpack(encoded.type_hash(), encoded.wire()).unwrap();
    assert_eq!(copied.prop_f32("blur"), Some(0.6));
    let mut store = DrawStore::default();
    store.apply(original.clone(), 0.0).unwrap();
    store.apply(DrawEntity::update("glass", "trail").with_blur(0.0).updating(&["props"]), 1.0).unwrap();
    let live = &store.get("glass", "trail").unwrap().entity;
    assert_eq!(live.prop_f32("blur"), Some(0.0));
    assert_eq!((&live.points, live.color, live.width, live.glow), (&original.points, original.color, original.width, original.glow));
    for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(store.apply(DrawEntity::update("glass", "trail").with_blur(value).updating(&["props"]), 2.0).is_err());
    }
    assert_eq!(store.get("glass", "trail").unwrap().entity.prop_f32("blur"), Some(0.0));
}

#[test]
fn scalar_updates_preserve_every_unspecified_header_and_section() {
    let base = DrawEntity::lines("lab", "energy", &[[0.0, 0.0, 0.0], [2.0, 3.0, 1.0]])
        .at_yaw([4.0, 5.0, 6.0], 0.5).with_scale([1.0, 2.0, 3.0])
        .with_color(rgba(70, 180, 220, 180)).with_color2(rgba(220, 80, 20, 200))
        .with_style(draw_style::LIGHT_WALL).with_width(0.25)
        .with_glow(0.6).with_pulse(0.8).with_flow(1.2, 2.0).with_beam(3.0)
        .with_fade(0.2).with_urgency(0.4).with_colormap(colormap::TURBO, -2.0, 10.0)
        .with_flags(draw_flag::FLAT | draw_flag::ON_TOP).with_lifetime(60.0)
        .with_labels(&["retained"]).with_prop("panel_finish", "black");
    let cases: &[(&str, fn(&mut DrawEntity))] = &[
        ("tint", |e| e.color = 0), ("color2", |e| e.color2 = 0),
        ("glow", |e| e.glow = 0.0), ("pulse_hz", |e| e.pulse_hz = 0.0),
        ("flow_speed", |e| e.flow_speed = 0.0), ("dash", |e| e.dash = 0.0),
        ("beam_height", |e| e.beam_height = 0.0), ("fade", |e| e.fade = 0.0),
        ("urgency", |e| e.urgency = 0.0), ("palette", |e| e.colormap = 0),
        ("vmin", |e| e.vmin = 0.0), ("vmax", |e| e.vmax = 0.0),
        ("props", |_| {}),
    ];
    for (mask, change) in cases {
        let mut store = DrawStore::default();
        store.apply(base.clone(), 0.0).unwrap();
        let msg = DrawEntity::update("lab", "energy").updating(&[mask]);
        store.apply(msg, 1.0).unwrap();
        let mut expected = base.clone();
        change(&mut expected);
        let live = &store.get("lab", "energy").unwrap().entity;
        assert_eq!(pack(live).wire(), pack(&expected).wire(), "selector {mask}");
    }
    let mut store = DrawStore::default();
    store.apply(base.clone(), 0.0).unwrap();
    store.apply(DrawEntity::update("lab", "energy").with_glow(-1.0)
        .with_prop("panel_finish", "white").updating(&["glow", "props"]), 1.0).unwrap();
    let expected = base.with_glow(-1.0).with_prop("panel_finish", "white");
    assert_eq!(pack(&store.get("lab", "energy").unwrap().entity).wire(), pack(&expected).wire());
}

#[test]
fn retained_generation_changes_only_when_the_entity_is_replaced() {
    let mut store = DrawStore::default();
    let source = DrawEntity::lines("t", "s", &[[0.0; 3], [1.0; 3]]).with_flags(draw_flag::SNAPSHOT);
    store.apply(source.clone(), 0.0).unwrap();
    let generation = store.get("t", "s").unwrap().generation;
    let mut extend = DrawEntity::lines("t", "s", &[[2.0; 3]]);
    extend.op = draw_op::EXTEND;
    for operation in [DrawEntity::update("t", "s").with_glow(0.3).updating(&["glow"]),
        DrawEntity::patch(draw_kind::LINES, "t", "s", 0).with_points(&[[0.5; 3]]), extend] {
        store.apply(operation, 1.0).unwrap();
        let item = store.get("t", "s").unwrap();
        assert_eq!(item.generation, generation);
        assert!(item.revision > generation);
    }
    store.apply(source, 2.0).unwrap();
    let replaced = store.get("t", "s").unwrap();
    assert!(replaced.generation > generation);
    assert_eq!(replaced.generation, replaced.revision);
    let generation = replaced.generation;
    let replacement = DrawEntity::points("t", "s", &vec![[0.0; 3]; 3000]);
    let chunks = replacement.chunks(DRAW_CHUNK_BYTES);
    for (i, chunk) in chunks.iter().enumerate() {
        store.apply(chunk.clone(), 3.0).unwrap();
        assert_eq!(store.get("t", "s").unwrap().generation == generation, i + 1 < chunks.len());
    }
}

#[test]
fn legacy_update_groups_and_empty_mask_still_replace_their_whole_group() {
    for mask in [Some("color"), Some("effects"), Some("colormap"), Some(""), None] {
        let mut store = DrawStore::default();
        let base = DrawEntity::points("lab", "p", &[[1.0, 2.0, 3.0]])
            .with_color(11).with_color2(22).with_glow(0.5).with_pulse(2.0)
            .with_flow(3.0, 4.0).with_beam(5.0).with_fade(0.6).with_urgency(0.7)
            .with_colormap(colormap::VIRIDIS, -10.0, 20.0);
        store.apply(base, 0.0).unwrap();
        let mut msg = DrawEntity::update("lab", "p");
        if let Some(mask) = mask { msg = msg.with_prop("set", mask); }
        store.apply(msg, 1.0).unwrap();
        let live = &store.get("lab", "p").unwrap().entity;
        let all = mask.is_none() || mask == Some("");
        assert_eq!((live.color, live.color2), if all || mask == Some("color") { (0, 0) } else { (11, 22) });
        assert_eq!([live.glow, live.pulse_hz, live.flow_speed, live.dash, live.beam_height, live.fade, live.urgency],
            if all || mask == Some("effects") { [0.0; 7] } else { [0.5, 2.0, 3.0, 4.0, 5.0, 0.6, 0.7] });
        assert_eq!((live.colormap, live.vmin, live.vmax),
            if all || mask == Some("colormap") { (0, 0.0, 0.0) } else { (colormap::VIRIDIS, -10.0, 20.0) });
        assert_eq!(live.point_at(0), [1.0, 2.0, 3.0]);
    }
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
    assert_eq!(wp.len(), 5);
    assert_eq!(wp[3].style, draw_style::PLAIN);
    assert_eq!(wp[3].props().get("label_selected").as_deref(), Some("true"));
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
    assert_eq!(serde_json::from_str::<[String; 2]>(&det[1].label_list()[0]).unwrap(), ["car", "score 0.92 / track #17"]);
    assert_eq!(det[0].colors[0] & 0x00ff_ffff, draw_color::ORANGE & 0x00ff_ffff);
    assert!(det.iter().all(|e| e.validate().is_ok()));
}

#[test]
fn covariance_dimension_changes_remove_obsolete_components() {
    let mut store = DrawStore::default();
    let mut six = vec![0.0; 36];
    (six[0], six[7], six[14], six[35]) = (0.25, 1.0, 0.5, 0.09);
    let two = vec![0.25, 0.0, 0.0, 1.0];
    let three = vec![0.25, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.5];
    for (cov, expected) in [(&six, vec!["ellipsoid", "yaw"]), (&two, vec!["mean", "rings"]),
        (&three, vec!["ellipsoid"]), (&two, vec!["mean", "rings"]), (&six, vec!["ellipsoid", "yaw"])] {
        for entity in w::covariance(&Place::default(), "n", "cov", Pose::default(), cov, &[1.0, 2.0], 0).unwrap() {
            store.apply(entity, 0.0).unwrap();
        }
        let mut ids: Vec<_> = store.iter().map(|(_, item)| item.entity.id()).collect();
        ids.sort();
        assert_eq!(ids, expected);
    }
}

#[test]
fn odometry_without_covariance_removes_only_its_covariance_parts() {
    let mut store = DrawStore::default();
    for ns in ["n/odom/cov", "n/odom/cov/nested", "n/odometry/cov"] {
        store.apply(DrawEntity::points(ns, "custom", &[[0.0; 3]]), 0.0).unwrap();
    }
    let mut six = vec![0.0; 36];
    (six[0], six[7], six[14], six[35]) = (0.25, 1.0, 0.5, 0.09);
    for cov in [&[0.25, 0.0, 0.0, 1.0][..], &six, &[0.25, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.5]] {
        for values in [cov, &[]] {
            for entity in w::odometry(&Place::default(), "n", "odom", Pose::default(), [1.0, 0.0, 0.0], values) {
                store.apply(entity, 0.0).unwrap();
            }
            let mut parts: Vec<_> = store.iter().filter(|(_, e)| e.entity.ns() == "n/odom/cov")
                .map(|(_, e)| e.entity.id()).collect();
            parts.sort();
            if values.is_empty() { assert_eq!(parts, ["custom"]); }
            else { assert!(parts.len() > 1); }
            assert_eq!(store.iter().filter(|(_, e)| e.entity.id() == "custom").count(), 3);
            assert_eq!(store.iter().filter(|(_, e)| e.entity.ns() == "n/odom").count(), 2);
        }
    }
}

#[test]
fn station_and_bracket_composites_share_geometry_and_lifecycle() {
    let place = Place::frame("robot/link").lifetime(7.0);
    let positions = [[0.0; 3], [3.0, 0.0, 0.0]];
    let waypoint = w::waypoints(&place, "n", "route", &positions, 1);
    let station = w::route_stations(&place, "n", "reference", &positions, 3.0, 0.45, draw_color::PLAN);
    assert_eq!(waypoint[2].points, station[1].points);
    assert_eq!(waypoint[1].kind, draw_kind::POINTS);
    assert_eq!(waypoint[1].colors[1], 1);
    assert_eq!(waypoint[3].label_list(), ["2"]);
    assert_eq!(waypoint[4].point_count(), 16);
    let pose = Pose::yawed(3.0, 4.0, 5.0, 0.3);
    let size = [2.0, 4.0, 6.0];
    let detection = w::Detection3d {
        pose, size, class: "fixture".into(), score: 0.82, track: Some(17), velocity: None, trail: Vec::new(),
    };
    let detections = w::detections_3d(&place, "n", "detections", &[detection], &w::DetectionStyle::default());
    assert_eq!(detections[0].instance_list()[0].size, size);
    assert_eq!(detections[2].op, draw_op::DELETE);
    assert_eq!(detections[1].kind, draw_kind::TEXT);
    assert_eq!(serde_json::from_str::<[String; 2]>(&detections[1].label_list()[0]).unwrap(), ["fixture", "score 0.82 / track #17"]);
    assert_eq!(detections[1].props().get("label_parts").as_deref(), Some("identity-detail-json"));
    assert_eq!(detections[1].props().get("label_neutral_identity").as_deref(), Some("true"));
    assert_eq!(detections[1].width, -16.0);
    let mut store = DrawStore::default();
    for e in waypoint.into_iter().chain(detections) {
        assert_eq!(e.props().get("frame").as_deref(), Some("robot/link"));
        assert_eq!(e.lifetime_s, 7.0);
        assert_eq!(e.glow, 0.0);
        assert_eq!(e.flags & draw_flag::ON_TOP, 0);
        store.apply(e, 0.0).unwrap();
    }
    for e in w::waypoints(&place, "n", "route", &[], 0).into_iter()
        .chain(w::detections_3d(&place, "n", "detections", &[], &w::DetectionStyle::default())) {
        assert_eq!(e.op, draw_op::DELETE);
        store.apply(e, 1.0).unwrap();
    }
    assert!(store.is_empty());
    let absent = w::waypoints(&place, "n", "route", &positions, 5);
    assert!(absent[3..].iter().all(|e| e.op == draw_op::DELETE));
}

#[test]
fn structural_helpers_share_corner_weight_without_changing_data() {
    let place = Place::frame("robot/link").lifetime(7.0);
    let pose = Pose::yawed(3.0, 4.0, 5.0, 0.3);
    let clearance = w::clearance_box(&place, "n", "bounds", pose, [2.0, 4.0, 6.0], 0x8050c080, "Supplied extent");
    assert_eq!(clearance[0].width, -w::STRUCTURE_CORNER_GAIN);
    assert_eq!(clearance[0].point_count(), 48);
    assert_eq!(clearance[0].color, 0x8050c080);
    let route = w::waypoints(&place, "n", "route", &[[0.0; 3], [3.0, 0.0, 0.0]], 1);
    assert_eq!(route[4].width, clearance[0].width);
    assert_eq!(route[2].width, -1.25);
    let detection = w::detections_3d(&place, "n", "det", &[w::Detection3d {
        pose, size: [2.0, 4.0, 6.0], class: "car".into(), score: 0.82, track: Some(17), velocity: None, trail: Vec::new(),
    }], &w::DetectionStyle { only_edge: true, line_width: -3.0, ..Default::default() });
    assert_eq!(detection[0].width, -3.0);
    assert_eq!(detection[0].style, draw_style::PLAIN);
    assert_eq!(detection[0].flags & draw_flag::FILL_MASK, draw_flag::FILL_WIRE);
    assert_eq!(detection[0].props().get("corner_frame").as_deref(), Some("true"));
    assert_eq!(detection[0].instance_list()[0].size, [2.0, 4.0, 6.0]);
    let image = w::detections_2d(&place, "n", "image", Pose::default(), 1.0, 0.8, [640.0, 480.0],
        &[[320.0, 240.0, 100.0, 80.0]], &["car".into()], &[0.82], 2.0);
    assert_eq!(image[0].props().get("corner_frame").as_deref(), Some("true"));
    assert_eq!(image[0].width, -1.0);
    assert_eq!(image[0].point_count(), 4);
    let points = image[0].point_list();
    let y = 2.0 * 0.5_f32.tan() * 100.0 / 640.0;
    let z = 2.0 * 0.4_f32.tan() * 80.0 / 480.0;
    for (actual, expected) in points.iter().zip([[2.0, y, z], [2.0, -y, z], [2.0, -y, -z], [2.0, y, -z]]) {
        assert!(actual.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6));
    }
    assert_eq!(image[0].colors, vec![w::class_color("car", &[]); 4]);
    assert_eq!(image[1].point_count(), 12);
    for entity in clearance.iter().chain(route.iter()).chain(detection.iter()).chain(image.iter()) {
        assert_eq!(entity.frame(), DrawFrame::Link("robot".into(), "link".into()));
        assert_eq!(entity.lifetime_s, 7.0);
        entity.validate().unwrap();
    }
}

#[test]
fn callout_and_frame_labels_use_world_anchors_without_art_glow() {
    let place = Place::default().no_drape();
    let callout = w::callout(&place, "n", "callout", [1.0, 2.0, 3.0], [0.0, 0.0, 1.0], "Supplied", draw_color::MAGENTA);
    assert_eq!(callout[0].point_list(), [[1.0, 2.0, 3.0], [1.0, 2.0, 4.0]]);
    assert_eq!(callout[1].kind, draw_kind::LINES);
    assert_eq!(callout[1].sub, line_mode::LIST);
    assert_eq!(callout[1].point_count(), 8);
    assert_eq!(callout[2].width, -16.0);
    let frame = w::frame(&place, "n", "frame", Pose::default(), None, 0.5, Some("Supplied frame"));
    assert_eq!(frame[1].kind, draw_kind::TEXT);
    assert_eq!(frame[1].props().get("label_icon").as_deref(), Some("frame"));
    for e in callout.iter().chain(frame.iter()) {
        assert_eq!(e.glow, 0.0);
        e.validate().unwrap();
    }
}

#[test]
fn detection_identity_detail_survives_label_chunking() {
    let classes = ["fixture\nupper", "value\u{1f}with separator", "quote \" and \\ slash", "点検センサー"];
    let dets: Vec<_> = (0..100).map(|i| w::Detection3d {
        pose: Pose::at(i as f64, 0.0, 0.0), size: [1.0; 3], class: classes[i % classes.len()].into(),
        score: 0.5, track: None, velocity: None, trail: Vec::new(),
    }).collect();
    let items = w::detections_3d(&Place::default().no_drape(), "n", "detections", &dets, &w::DetectionStyle::default());
    let tags = &items[1];
    let chunks = tags.chunks(512);
    assert!(chunks.len() > 1);
    let mut store = DrawStore::default();
    for chunk in chunks {
        chunk.validate().unwrap();
        assert_eq!(chunk.props().get("label_parts").as_deref(), Some("identity-detail-json"));
        store.apply(chunk, 0.0).unwrap();
    }
    let restored = &store.get("n/detections", "tags").unwrap().entity;
    let labels = restored.label_list();
    assert_eq!(labels.len(), dets.len());
    for (i, label) in labels.iter().enumerate() {
        assert_eq!(serde_json::from_str::<[String; 2]>(label).unwrap(), [classes[i % classes.len()], "score 0.50"]);
    }
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

#[test]
fn generated_color_overrides_keep_semantic_rgb_and_multiply_opacity() {
    let p = Place::default().no_drape();
    let points = [[0.0; 3], [2.0, 0.0, 0.0], [4.0, 0.0, 0.0]];
    let poses = [Pose::default(), Pose::at(2.0, 0.0, 0.0)];
    for tint in [draw_color::MAGENTA, draw_color::with_alpha(draw_color::GREEN, 0.5), 0] {
        let routes = w::waypoints_colored(&p, "n", "route", &points, 1, tint);
        assert_eq!(routes[0].colors[2], tint.max(1));
        assert_eq!(routes[0].colors[0] & 0x00ff_ffff, draw_color::WHITE & 0x00ff_ffff);
        assert_eq!(routes[3].color & 0x00ff_ffff, draw_color::GOAL & 0x00ff_ffff);
        let swaths = w::swaths_colored(&p, "n", "swaths", &[points.to_vec(), points.to_vec()], 3.0, 1, tint);
        assert_eq!(swaths[0].color, tint.max(1));
        assert_eq!(swaths[1].color & 0x00ff_ffff, draw_color::HISTORY & 0x00ff_ffff);
        let compass = w::compass_colored(&p, "n", "compass", [0.0; 3], 4.0, tint);
        assert_eq!(compass[2].colors[1..], [tint.max(1); 3]);
        assert_eq!(compass[2].colors[0] & 0x00ff_ffff, draw_color::GOAL & 0x00ff_ffff);
        let particles = w::particles_colored(&p, "n", "particles", &poses, &[0.2, 1.0], 0.1, 0.5, w::Glyph::Arrow, tint);
        let angle = w::angle_colored(&p, "n", "angle", [0.0; 3], 0.0, 1.0, 1.0, tint);
        for e in routes.iter().chain(&swaths).chain(&compass).chain(std::iter::once(&particles)).chain(&angle) {
            e.validate().unwrap();
            for c in if e.colors.is_empty() { std::slice::from_ref(&e.color) } else { &e.colors } {
                if *c != 0 { assert!(c >> 24 <= tint >> 24); }
            }
        }
        assert!(particles.colors.iter().all(|c| c & 0x00ff_ffff == tint.max(1) & 0x00ff_ffff));
        let covariance = w::covariance(&p, "n", "covariance", Pose::default(), &[1.0, 0.0, 0.0, 1.0], &[1.0, 2.0], tint.max(1)).unwrap();
        assert!(covariance[0].colors.iter().all(|c| c >> 24 <= tint >> 24));
        let callout = w::callout(&p, "n", "callout", [0.0; 3], [0.0, 0.0, 1.0], "Supplied", tint.max(1));
        assert_eq!(callout[2].color >> 24, tint >> 24);
        assert_eq!(callout[2].color & 0x00ff_ffff, 0x00ff_ffff);
    }
    assert_eq!(w::waypoints(&p, "n", "route", &points, 1)[0].colors,
        [draw_color::with_alpha(draw_color::WHITE, 0.35), draw_color::PLAN, draw_color::PLAN]);
    assert_eq!(w::swaths(&p, "n", "swaths", &[points.to_vec()], 3.0, 0)[0].color, draw_color::PLAN);
}

#[test]
fn data_and_caller_item_palettes_remain_authoritative() {
    let p = Place::default();
    let bars = w::panel(&p, "n", "bars", [0.0; 3], panel_kind::BARS, &[0.2, 0.8], &[], "Fixture")
        .with_colors(&[draw_color::GREEN, draw_color::ORANGE]).with_color(draw_color::MAGENTA);
    assert_eq!(bars.colors, [draw_color::GREEN, draw_color::ORANGE]);
    assert_eq!(bars.values, [0.2, 0.8]);
    let cloud = w::cloud(&p, "n", "cloud", &[[0.0; 3], [1.0, 0.0, 0.0]], &[0.1, 0.9], colormap::VIRIDIS, -1.5, 0.0)
        .with_color(draw_color::MAGENTA);
    assert_eq!(cloud.colormap, colormap::VIRIDIS);
    assert_eq!(cloud.values, [0.1, 0.9]);
    let det = w::Detection3d { pose: Pose::default(), size: [1.0; 3], class: "fixture".into(), score: 0.5,
        track: None, velocity: None, trail: Vec::new() };
    let style = w::DetectionStyle { class_colors: vec![("fixture".into(), 0)], ..Default::default() };
    let dets = w::detections_3d(&p, "n", "detections", &[det], &style);
    assert_eq!(dets[0].colors, [1]);
    assert_eq!(dets[1].colors, [1]);
}
