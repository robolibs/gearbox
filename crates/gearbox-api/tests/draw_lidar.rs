use gearbox_api::wire::draw_data::lidar::{Completion, LidarAssembler, LidarLimits, LidarSweep};
use gearbox_api::wire::draw_data::{
    CaptureConvention, CaptureStamp, CaptureTransform, ColumnView, DatumSnapshot,
    NATIVE_CAPTURE_PROP, NativeLidarCapture, Placement, RigidPose,
};
use gearbox_api::{LidarScan, Props};

const ROWS: u32 = 4;
const COLS: u32 = 6;

fn capture(sim_time: f64, sample: u64) -> NativeLidarCapture {
    NativeLidarCapture {
        revision: 1,
        machine: "rover".into(),
        link: "lidar".into(),
        convention: CaptureConvention::RightHandedZUpMetres,
        transform: CaptureTransform {
            revision: 1,
            source_frame: "rover/lidar".into(),
            target_frame: "world".into(),
            stamp: CaptureStamp {
                simulation_time_s: sim_time,
                source_time_ns: None,
                source_clock: None,
                sample,
                scene_epoch: 3,
            },
            pose: RigidPose {
                translation_m: [1.0, 2.0, 0.5],
                rotation_wxyz: [1.0, 0.0, 0.0, 0.0],
            },
            datum: Some(DatumSnapshot {
                origin_ecef_m: [3.9e6, 3.4e5, 5.0e6],
                axes_to_ecef_wxyz: [1.0, 0.0, 0.0, 0.0],
            }),
        },
    }
}

/// Pulse `i` ranges `1 + i/10` m along x, except every fifth misses (inf) and
/// every seventh is NaN.
fn pulse(i: usize) -> (f32, [f32; 3]) {
    if i % 5 == 4 {
        (f32::INFINITY, [f32::NAN; 3])
    } else if i % 7 == 6 {
        (f32::NAN, [f32::NAN; 3])
    } else {
        let r = 1.0 + i as f32 / 10.0;
        (r, [r, 0.0, 0.0])
    }
}

fn chunk(offset: usize, count: usize, sim_time: f64, sample: u64, with_capture: bool) -> LidarScan {
    let (ranges, points): (Vec<f32>, Vec<[f32; 3]>) = (offset..offset + count).map(pulse).unzip();
    let mut scan = LidarScan {
        sim_time_s: sim_time,
        theta_min: 0.1,
        theta_max: 3.0,
        phi_min: -3.1,
        phi_max: 3.1,
        max_range_m: 20.0,
        link_index: 2,
        rows: ROWS,
        cols: COLS,
        pulse_offset: offset as u32,
        pulse_count: count as u32,
        hits: ranges.iter().filter(|r| r.is_finite()).count() as u32,
        stamp_ms: 77,
        sample: sample as u32,
        ranges,
        props: Props::from_pairs(&[("name", "lidar")]).into_bytes(),
        ..Default::default()
    };
    scan.set_points(&points);
    if with_capture {
        scan.set_native_capture(&capture(sim_time, sample)).unwrap();
    }
    scan
}

fn assembler() -> LidarAssembler {
    LidarAssembler::new(LidarLimits::default(), Completion::CompleteOnly)
}

fn valid(view: ColumnView<'_>, i: usize) -> bool {
    view.validity.is_empty() || view.validity[i / 8] & (1 << (i % 8)) != 0
}

#[test]
fn out_of_order_chunks_make_one_organized_sweep_with_explicit_invalid_misses() {
    let mut lidar = assembler();
    assert!(lidar.push("rover", &chunk(16, 8, 2.5, 9, true)).unwrap().is_empty());
    assert!(lidar.push("rover", &chunk(0, 10, 2.5, 9, true)).unwrap().is_empty());
    let done = lidar.push("rover", &chunk(10, 6, 2.5, 9, true)).unwrap();
    assert_eq!(done.len(), 1);
    let sweep = &done[0];
    assert!(sweep.complete() && sweep.missing().is_empty());
    assert_eq!((sweep.rows, sweep.cols, sweep.pulses()), (ROWS, COLS, 24));
    assert_eq!(sweep.capture, Some(capture(2.5, 9)));

    let schema = LidarSweep::schema();
    let pages = sweep.pages();
    assert_eq!(pages.len(), 1);
    let page = &pages[0];
    page.validate(&schema).unwrap();
    page.encode(&schema).unwrap();
    for i in 0..24 {
        let (range, _) = pulse(i);
        let id = u64::from_le_bytes(page.columns[1].values[8 * i..8 * i + 8].try_into().unwrap());
        assert_eq!(id, i as u64, "pulse IDs are organized indices");
        let ring = u16::from_le_bytes(page.columns[3].values[2 * i..2 * i + 2].try_into().unwrap());
        assert_eq!(ring as usize, i / COLS as usize);
        let hit = range.is_finite();
        assert_eq!(valid(page.columns[0].as_view(), i), hit);
        assert_eq!(valid(page.columns[2].as_view(), i), hit);
        let stored = f32::from_le_bytes(page.columns[2].values[4 * i..4 * i + 4].try_into().unwrap());
        assert_eq!(stored, if hit { range } else { 0.0 });
        if !hit {
            assert!(page.columns[0].values[12 * i..12 * i + 12].iter().all(|&b| b == 0));
        }
    }

    let snapshot = sweep.scan_descriptor(41, Placement::Snapshot).unwrap();
    assert_eq!(snapshot.source_count, 24);
    let organization = snapshot.organization.unwrap();
    assert_eq!((organization.width, organization.height), (COLS, ROWS));
    assert_eq!(snapshot.frame.frame, "rover/lidar");
    assert_eq!(snapshot.frame.capture_transform, Some(capture(2.5, 9).transform));
    let live = sweep.scan_descriptor(41, Placement::Live).unwrap();
    assert_eq!(live.frame.capture_transform, None);
    assert_eq!(live.frame.capture, Some(capture(2.5, 9).transform.stamp));
}

#[test]
fn a_sweep_without_capture_can_only_be_placed_on_its_live_link() {
    let mut lidar = assembler();
    let sweep = lidar.push("rover", &chunk(0, 24, 1.0, 4, false)).unwrap().remove(0);
    assert_eq!(sweep.capture, None);
    let error = sweep.scan_descriptor(1, Placement::Snapshot).unwrap_err();
    assert!(error.contains("snapshot placement is unavailable"), "{error}");
    let live = sweep.scan_descriptor(1, Placement::Live).unwrap();
    assert_eq!((live.frame.capture.clone(), live.frame.placement), (None, Placement::Live));
}

#[test]
fn a_wrapped_sample_counter_is_a_new_sweep_not_a_merge_or_a_new_map() {
    let mut lidar = assembler();
    let late = u64::from(u32::MAX);
    assert!(lidar.push("rover", &chunk(0, 12, 10.0, late, true)).unwrap().is_empty());
    assert!(lidar.push("rover", &chunk(0, 12, 10.02, late + 1, true)).unwrap().is_empty());
    assert_eq!(lidar.open(), 2);
    let first = lidar.push("rover", &chunk(12, 12, 10.0, late, true)).unwrap().remove(0);
    let second = lidar.push("rover", &chunk(12, 12, 10.02, late + 1, true)).unwrap().remove(0);
    assert_eq!((first.sample, second.sample), (late, late + 1));
    assert_eq!(first.source_frame(), second.source_frame());
}

#[test]
fn malformed_chunks_fail_without_touching_the_open_sweep() {
    let mut lidar = assembler();
    lidar.push("rover", &chunk(0, 6, 3.0, 1, false)).unwrap();
    let mut cases: Vec<(&str, LidarScan)> = Vec::new();
    let mut bad = chunk(6, 6, 3.0, 1, false);
    bad.rows = 0;
    cases.push(("empty dimensions", bad));
    let mut bad = chunk(6, 6, 3.0, 1, false);
    (bad.rows, bad.cols) = (u32::MAX, u32::MAX);
    cases.push(("huge dimensions", bad));
    let mut bad = chunk(6, 6, 3.0, 1, false);
    bad.points.pop();
    cases.push(("short points", bad));
    let mut bad = chunk(6, 6, 3.0, 1, false);
    bad.pulse_offset = 20;
    cases.push(("past the sweep", bad));
    let mut bad = chunk(6, 6, 3.0, 1, false);
    bad.ranges[0] = -1.0;
    cases.push(("negative range", bad));
    let mut bad = chunk(6, 6, 3.0, 1, false);
    bad.ranges[0] = 25.0;
    cases.push(("beyond max range", bad));
    let mut bad = chunk(6, 6, 3.0, 1, false);
    bad.points[3] = f32::NAN;
    cases.push(("hit without a point", bad));
    let mut bad = chunk(6, 6, 3.0, 1, false);
    bad.hits += 1;
    cases.push(("hit count", bad));
    let mut bad = chunk(6, 6, 3.0, 1, false);
    bad.theta_max = 2.0;
    cases.push(("metadata disagreement", bad));
    cases.push(("overlap", chunk(3, 6, 3.0, 1, false)));
    let mut bad = chunk(6, 6, 3.0, 1, false);
    bad.props = Props::from_pairs(&[("name", "lidar"), (NATIVE_CAPTURE_PROP, "{not json")]).into_bytes();
    cases.push(("malformed capture", bad));
    for (case, bad) in cases {
        assert!(lidar.push("rover", &bad).is_err(), "{case} was accepted");
    }
    assert!(lidar.push("other", &chunk(0, 6, 3.0, 1, true)).is_err(), "capture of another machine");
    assert_eq!(lidar.open(), 1);
    let rest = lidar.push("rover", &chunk(6, 18, 3.0, 1, false)).unwrap();
    assert!(rest[0].complete());
}

#[test]
fn incomplete_sweeps_are_dropped_by_default_and_exposed_with_missing_pulses_on_request() {
    let limits = LidarLimits { max_open: 1, ..Default::default() };
    let mut strict = LidarAssembler::new(limits, Completion::CompleteOnly);
    strict.push("rover", &chunk(0, 10, 1.0, 1, false)).unwrap();
    assert!(strict.push("rover", &chunk(0, 10, 1.1, 2, false)).unwrap().is_empty());
    assert_eq!(strict.abandoned, 1);
    assert!(strict.flush().is_empty());
    assert_eq!(strict.abandoned, 2);

    let mut partial = LidarAssembler::new(limits, Completion::Partial);
    partial.push("rover", &chunk(0, 10, 1.0, 1, false)).unwrap();
    let pushed_out = partial.push("rover", &chunk(14, 10, 1.1, 2, false)).unwrap();
    assert_eq!(pushed_out.len(), 1);
    let sweep = &pushed_out[0];
    assert!(!sweep.complete());
    assert_eq!(sweep.missing(), (10..24).collect::<Vec<u32>>());
    let page = &sweep.pages()[0];
    page.validate(&LidarSweep::schema()).unwrap();
    assert!((10..24).all(|i| !valid(page.columns[0].as_view(), i)));
    let left = partial.flush();
    assert_eq!(left[0].missing(), (0..14).collect::<Vec<u32>>());
}

#[test]
fn a_large_sweep_splits_into_bounded_pages_that_keep_global_pulse_ids() {
    let (rows, cols) = (128u32, 1024u32);
    let mut lidar = assembler();
    let mut done = Vec::new();
    for offset in (0..(rows * cols) as usize).step_by(900) {
        let count = 900.min((rows * cols) as usize - offset);
        let mut part = chunk(0, count, 5.0, 8, false);
        (part.rows, part.cols, part.pulse_offset, part.max_range_m) = (rows, cols, offset as u32, 200.0);
        done.extend(lidar.push("rover", &part).unwrap());
    }
    let sweep = done.remove(0);
    let pages = sweep.pages();
    assert_eq!(pages.len(), 2);
    let schema = LidarSweep::schema();
    let second = &pages[1];
    second.validate(&schema).unwrap();
    let first_id = u64::from_le_bytes(second.columns[1].values[..8].try_into().unwrap());
    assert_eq!(first_id, 65_536);
    assert_eq!(pages.iter().map(|p| p.count as u64).sum::<u64>(), u64::from(rows * cols));
}

#[test]
fn a_self_filter_turns_near_returns_into_invalid_observations_in_place() {
    let mut lidar = assembler();
    let mut sweep = lidar.push("rover", &chunk(0, 24, 1.0, 4, false)).unwrap().remove(0);
    let near: Vec<usize> = (0..24).filter(|&i| pulse(i).0.is_finite() && pulse(i).0 < 2.0).collect();
    assert_eq!(sweep.drop_near(2.0), near.len());
    let page = &sweep.pages()[0];
    page.validate(&LidarSweep::schema()).unwrap();
    for i in 0..24 {
        let kept = pulse(i).0.is_finite() && !near.contains(&i);
        assert_eq!(valid(page.columns[0].as_view(), i), kept);
        let id = u64::from_le_bytes(page.columns[1].values[8 * i..8 * i + 8].try_into().unwrap());
        assert_eq!(id, i as u64);
    }
    assert_eq!(sweep.scan_descriptor(1, Placement::Live).unwrap().source_count, 24);
}
