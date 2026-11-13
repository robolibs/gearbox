use gearbox_api::wire::{draw_data::*, pack, unpack};
use gearbox_api::{LIDAR_CHUNK_PULSES, LidarScan, MAX_PAYLOAD_BYTES, Props};

#[allow(dead_code)]
#[path = "support/draw_data_contracts.rs"]
mod contracts;

fn capture() -> NativeLidarCapture {
    NativeLidarCapture {
        revision: 1,
        machine: "robot".into(),
        link: "lidar".into(),
        convention: CaptureConvention::RightHandedZUpMetres,
        transform: contracts::scan(1).frame.capture_transform.unwrap(),
    }
}

fn scan(count: usize) -> LidarScan {
    let capture = capture();
    LidarScan {
        sim_time_s: capture.transform.stamp.simulation_time_s,
        sample: capture.transform.stamp.sample as u32,
        rows: 1,
        cols: count as u32,
        pulse_count: count as u32,
        ranges: vec![1.0; count],
        points: vec![1.0; 3 * count],
        props: Props::from_pairs(&[("name", "lidar"), ("extra", "preserved")]).into_bytes(),
        ..Default::default()
    }
}

#[test]
fn optional_capture_preserves_lidar_v1_layout_and_unrelated_props() {
    let mut scan = scan(4);
    assert_eq!(scan.native_capture().unwrap(), None);
    let before = pack(&scan);
    scan.set_native_capture(&capture()).unwrap();
    let after = pack(&scan);
    assert_eq!(before.type_hash(), after.type_hash());
    let old_reader: LidarScan = unpack(after.type_hash(), after.wire()).unwrap();
    assert_eq!(old_reader.ranges, vec![1.0; 4]);
    assert_eq!(old_reader.points, vec![1.0; 12]);
    assert_eq!(old_reader.name(), "lidar");
    assert_eq!(
        old_reader.props().get("extra").as_deref(),
        Some("preserved")
    );
    assert_eq!(old_reader.native_capture().unwrap(), Some(capture()));
}

#[test]
fn all_chunks_share_exact_pose_datum_scene_epoch_and_full_u64_sample() {
    let mut capture = capture();
    capture.transform.stamp.sample = u64::MAX;
    capture.transform.stamp.scene_epoch = u64::MAX;
    capture.transform.stamp.source_time_ns = Some(u64::MAX);
    capture.transform.pose.translation_m = [1.25, 2.5, 3.75];
    let mut chunks = Vec::new();
    for offset in [0, 4, 8] {
        let mut chunk = scan(4);
        chunk.cols = 12;
        chunk.pulse_offset = offset;
        chunk.sample = u32::MAX;
        chunk.set_native_capture(&capture).unwrap();
        chunks.push(chunk);
    }
    for chunk in chunks {
        assert_eq!(chunk.native_capture().unwrap(), Some(capture.clone()));
    }
}

#[test]
fn capture_rejects_wrong_source_convention_revision_and_unanchored_pose() {
    let value = serde_json::to_value(capture()).unwrap();
    for (field, replacement) in [
        ("revision", 2.into()),
        ("convention", "physics_y_up".into()),
        ("machine", "another".into()),
        ("link", "other".into()),
    ] {
        let mut json = value.clone();
        json[field] = replacement;
        assert!(NativeLidarCapture::from_json(&json.to_string()).is_err());
    }
    let mut bad = capture();
    bad.transform.datum = None;
    assert!(bad.to_json().is_err());
    let mut json = value;
    json["unexpected"] = true.into();
    assert!(NativeLidarCapture::from_json(&json.to_string()).is_err());
    assert!(NativeLidarCapture::from_json(&" ".repeat(MAX_NATIVE_CAPTURE_BYTES + 1)).is_err());
}

#[test]
fn attaching_capture_is_atomic_when_headers_or_shape_disagree() {
    for mutate in [0, 1, 2, 3, 4] {
        let mut scan = scan(4);
        match mutate {
            0 => scan.sample += 1,
            1 => scan.sim_time_s += 1.0,
            2 => scan.props = Props::from_pairs(&[("name", "wrong")]).into_bytes(),
            3 => scan.pulse_count += 1,
            _ => scan.pulse_offset = u32::MAX,
        }
        let old_props = scan.props.clone();
        assert!(scan.set_native_capture(&capture()).is_err());
        assert_eq!(scan.props, old_props);
    }
}

#[test]
fn capture_accessor_rejects_malformed_maps_and_json_instead_of_silent_live_fallback() {
    let mut scan = scan(0);
    for props in [
        vec![1, 2, 3],
        vec![255; 16],
        vec![0; MAX_NATIVE_CAPTURE_PROPS_BYTES + 1],
        Props::from_pairs(&[("name", "lidar"), (NATIVE_CAPTURE_PROP, "{")]).into_bytes(),
    ] {
        scan.props = props;
        assert!(scan.native_capture().is_err());
    }
    scan.props = Props::from_entries((0..33).map(|i| (format!("key{i:02}"), "v"))).into_bytes();
    assert!(scan.native_capture().is_err());
    scan.props.clear();
    assert_eq!(scan.native_capture().unwrap(), None);
}

#[test]
fn real_wire_size_selects_pulse_capacity_with_large_metadata() {
    let mut capture = capture();
    capture.machine = "m".repeat(100);
    capture.link = "l".repeat(100);
    capture.transform.source_frame = format!("{}/{}", capture.machine, capture.link);
    let props = Props::from_pairs(&[
        ("name", &capture.link),
        (NATIVE_CAPTURE_PROP, &capture.to_json().unwrap()),
        ("other", &"x".repeat(1800)),
    ])
    .into_bytes();
    let count = lidar_chunk_capacity(&props).unwrap();
    assert!(count > 0 && count < LIDAR_CHUNK_PULSES);
    let mut scan = scan(count);
    scan.props = props.clone();
    scan.set_native_capture(&capture).unwrap();
    assert!(pack(&scan).wire().len() <= MAX_PAYLOAD_BYTES);
    scan.rows = 1;
    scan.cols += 1;
    scan.pulse_count += 1;
    scan.ranges.push(1.0);
    scan.points.extend([1.0; 3]);
    assert!(pack(&scan).wire().len() > MAX_PAYLOAD_BYTES);
    assert!(scan.set_native_capture(&capture).is_err());
    assert_eq!(scan.props, props);
}

#[test]
fn unchanged_legacy_metadata_keeps_the_original_900_pulse_capacity() {
    let scan = scan(LIDAR_CHUNK_PULSES);
    assert_eq!(
        lidar_chunk_capacity(&scan.props).unwrap(),
        LIDAR_CHUNK_PULSES
    );
    assert!(pack(&scan).wire().len() <= MAX_PAYLOAD_BYTES);
}

#[test]
fn json_capture_time_roundtrips_match_the_binary_header_exactly() {
    let mut seed = 0x987654321abcdefu64;
    for _ in 0..4096 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let time = (seed >> 11) as f64 / ((1u64 << 53) as f64) * 12345.6789;
        let mut capture = capture();
        capture.transform.stamp.simulation_time_s = time;
        let mut scan = scan(1);
        scan.sim_time_s = time;
        scan.set_native_capture(&capture).unwrap();
        let decoded = scan.native_capture().unwrap();
        assert_eq!(
            decoded.unwrap().transform.stamp.simulation_time_s.to_bits(),
            time.to_bits()
        );
    }
}
