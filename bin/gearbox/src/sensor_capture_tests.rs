use bevy::math::{DQuat, DVec3};
use gearbox_api::wire::{pack, unpack};
use gearbox_api::{LIDAR_CHUNK_PULSES, LidarScan, MAX_PAYLOAD_BYTES, Props, SimResetRequest, clear_scope};
use gearbox_globe::Datum;
use molla_math::{Quat as MQuat, Transform as MTransform, Vec3 as MVec3};

use super::*;

fn sensor(position: DVec3, rotation: DQuat) -> MTransform {
    MTransform::new(
        MVec3::new(position.x, position.y, position.z),
        MQuat::from_xyzw(rotation.x, rotation.y, rotation.z, rotation.w),
    )
}

/// A captured local point in ECEF, the way a snapshot consumer places it.
fn captured_ecef(capture: &CapturePose, local: DVec3) -> DVec3 {
    let [w, x, y, z] = capture.datum.axes_to_ecef_wxyz;
    let axes = DQuat::from_xyzw(x, y, z, w);
    let [w, x, y, z] = capture.pose.rotation_wxyz;
    let rotation = DQuat::from_xyzw(x, y, z, w);
    DVec3::from_array(capture.datum.origin_ecef_m)
        + axes * (rotation * local + DVec3::from_array(capture.pose.translation_m))
}

fn close(a: DVec3, b: DVec3) {
    assert!(a.distance(b) < 1e-6, "{a} != {b}");
}

#[test]
fn site_axes_become_enu_and_every_basis_vector_lands_where_the_site_put_it() {
    let datum = Datum::at(52.370216, 4.895168);
    let at = DVec3::new(3.0, 1.5, -2.0);
    let turn = DQuat::from_rotation_y(0.5) * DQuat::from_rotation_x(-0.3);
    let capture = CapturePose::of(&sensor(at, turn), (datum, at), 7);
    assert_eq!(capture.pose.translation_m, [-2.0, 3.0, 1.5]);
    assert_eq!(capture.scene_epoch, 7);
    for local in [DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z, DVec3::new(4.0, -1.0, 2.5)] {
        close(captured_ecef(&capture, local), datum.to_ecef(at + turn * local));
    }
    let east = datum.to_ecef(DVec3::Z) - datum.origin;
    let level = CapturePose::of(&sensor(DVec3::ZERO, DQuat::IDENTITY), (datum, DVec3::ZERO), 1);
    close(captured_ecef(&level, DVec3::ZERO), datum.origin);
    let [w, x, y, z] = level.datum.axes_to_ecef_wxyz;
    close(DQuat::from_xyzw(x, y, z, w) * DVec3::X, east);
}

#[test]
fn a_far_site_uses_its_own_region_offset_and_datum() {
    let datum = Datum::at(-33.8688, 151.2093);
    let local = DVec3::new(-12.0, 0.75, 40.0);
    let physics = gearbox_globe::physics_offset(2) + local;
    let (site, place) = crate::globe::site_local(physics.x, physics.y, physics.z);
    assert_eq!(site, 2);
    let place = DVec3::from_array(place);
    close(place, local);
    let turn = DQuat::from_rotation_y(-2.1);
    let capture = CapturePose::of(&sensor(physics, turn), (datum, place), 3);
    for point in [DVec3::X, DVec3::new(0.0, 2.0, -5.0)] {
        close(captured_ecef(&capture, point), datum.to_ecef(local + turn * point));
    }
}

#[test]
fn native_capture_names_its_source_frame_and_validates() {
    let datum = Datum::at(52.0, 5.0);
    let capture = CapturePose::of(&sensor(DVec3::ONE, DQuat::IDENTITY), (datum, DVec3::ONE), 9);
    let native = capture.native("tractor", "roof_lidar", 12.5, u64::from(u32::MAX) + 4);
    native.validate().unwrap();
    assert_eq!(native.transform.source_frame, "tractor/roof_lidar");
    assert_eq!(native.transform.stamp.scene_epoch, 9);
    assert_eq!(native.transform.stamp.sample, u64::from(u32::MAX) + 4);
    assert_eq!(native.transform.datum.as_ref(), Some(&capture.datum));
}

fn sweep_chunks(props: &[u8], capacity: usize, sim_time: f64, sample: u64, pulses: usize) -> Vec<LidarScan> {
    let ranges: Vec<f32> = (0..pulses).map(|i| if i % 7 == 0 { f32::INFINITY } else { i as f32 }).collect();
    let points: Vec<[f32; 3]> = (0..pulses).map(|i| [i as f32, -(i as f32), 0.5]).collect();
    ranges
        .chunks(capacity)
        .enumerate()
        .map(|(chunk, part)| {
            let offset = chunk * capacity;
            let mut scan = LidarScan {
                sim_time_s: sim_time,
                rows: 64,
                cols: (pulses / 64) as u32,
                pulse_offset: offset as u32,
                pulse_count: part.len() as u32,
                hits: part.iter().filter(|r| r.is_finite()).count() as u32,
                sample: sample as u32,
                ranges: part.to_vec(),
                props: props.to_vec(),
                ..Default::default()
            };
            scan.set_points(&points[offset..offset + part.len()]);
            scan
        })
        .collect()
}

#[test]
fn the_longest_capture_still_fits_every_chunk_and_old_readers_see_the_same_sweep() {
    let machine = "m".repeat(127);
    let link = "l".repeat(127);
    let datum = Datum::at(-89.9, -179.9);
    let at = DVec3::new(-1.0e5, 3.0e3, 7.5e4);
    let turn = DQuat::from_rotation_z(1.234) * DQuat::from_rotation_y(-0.42);
    let capture = CapturePose::of(&sensor(at, turn), (datum, at), u64::MAX)
        .native(&machine, &link, 1.0e9 + 0.123, u64::MAX);
    let (props, capacity, refused) = chunk_props(&link, Some(&capture));
    assert_eq!(refused, None);
    assert!(capacity > 0 && capacity <= LIDAR_CHUNK_PULSES);
    let (plain, plain_capacity, _) = chunk_props(&link, None);
    assert_eq!(plain_capacity, LIDAR_CHUNK_PULSES);
    let pulses = 64 * 1024;
    let chunks = sweep_chunks(&props, capacity, 1.0e9 + 0.123, u64::MAX, pulses);
    let old = sweep_chunks(&plain, plain_capacity, 1.0e9 + 0.123, u64::MAX, pulses);
    let mut covered = 0;
    for chunk in &chunks {
        let env = pack(chunk);
        assert!(env.wire().len() <= MAX_PAYLOAD_BYTES);
        let read: LidarScan = unpack(env.type_hash(), env.wire()).unwrap();
        assert_eq!(read.native_capture().unwrap().as_ref(), Some(&capture));
        assert_eq!(read.name(), link);
        assert_eq!(read.pulse_offset as usize, covered);
        covered += read.pulse_count as usize;
    }
    assert_eq!(covered, pulses);
    let ranges = |c: &[LidarScan]| c.iter().flat_map(|c| c.ranges.iter().map(|v| v.to_bits())).collect::<Vec<_>>();
    let points = |c: &[LidarScan]| c.iter().flat_map(|c| c.points.iter().map(|v| v.to_bits())).collect::<Vec<_>>();
    assert_eq!(ranges(&chunks), ranges(&old));
    assert_eq!(points(&chunks), points(&old));
    let hits = |chunks: &[LidarScan]| chunks.iter().map(|c| c.hits).sum::<u32>();
    assert_eq!(hits(&chunks), hits(&old));
    assert_eq!(pack(&old[0]).type_hash(), pack(&chunks[0]).type_hash());
}

#[test]
fn a_capture_that_cannot_encode_leaves_plain_chunks_and_says_why() {
    let datum = Datum::at(52.0, 5.0);
    let capture = CapturePose::of(&sensor(DVec3::ZERO, DQuat::IDENTITY), (datum, DVec3::ZERO), 1)
        .native("fleet/tractor", "lidar", 1.0, 1);
    let (props, capacity, refused) = chunk_props("lidar", Some(&capture));
    assert!(refused.is_some());
    assert_eq!(capacity, LIDAR_CHUNK_PULSES);
    assert_eq!(props, Props::from_pairs(&[("name", "lidar")]).into_bytes());
}

#[test]
fn a_machine_reset_starts_a_new_scene_epoch_and_a_marker_clear_does_not() {
    let mut app = bevy::app::App::new();
    app.add_message::<SimResetRequest>()
        .add_systems(bevy::app::Update, bump_scene_epoch_on_reset);
    let before = scene_epoch();
    app.world_mut()
        .write_message(SimResetRequest { scope: clear_scope::MARKERS, ..Default::default() });
    app.update();
    assert_eq!(scene_epoch(), before);
    app.world_mut()
        .write_message(SimResetRequest { scope: clear_scope::MACHINES, ..Default::default() });
    app.update();
    assert_eq!(scene_epoch(), before + 1);
    assert_ne!(scene_epoch(), 0);
}
