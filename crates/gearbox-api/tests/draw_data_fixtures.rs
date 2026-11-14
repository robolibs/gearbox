#[path = "support/draw_data_fixtures.rs"]
mod fixtures;

use fixtures::*;

#[test]
fn cloud_generators_preserve_ids_and_coexisting_attributes() {
    for shape in CLOUD_SHAPES {
        let samples = cloud(shape, 1024, FIXTURE_SEED).unwrap();
        assert_eq!(samples, cloud(shape, 1024, FIXTURE_SEED).unwrap());
        for (i, sample) in samples.iter().enumerate() {
            assert_eq!(sample.id, i as u64);
            assert!(sample.xyz.iter().all(|v| v.is_finite()));
            assert!(sample.intensity >= 0.0 && sample.intensity <= 1.0);
            assert_eq!(sample.class_id, (i % 4) as u32);
            assert_eq!(sample.rgba[3], 255);
        }
        let manifest = cloud_manifest(shape, FIXTURE_SEED, &samples);
        assert_eq!(manifest.count, 1024);
        assert_eq!(
            manifest.queries.iter().map(|p| p.id).collect::<Vec<_>>(),
            [0, 512, 1023]
        );
        assert_eq!(
            manifest.valid_count,
            if shape == CloudShape::NoisyScan {
                930
            } else {
                1024
            }
        );
    }
}

#[test]
fn analytical_fixture_queries_have_known_positions() {
    let plane = cloud(CloudShape::Plane, 257, FIXTURE_SEED).unwrap();
    assert_eq!(plane[0].xyz, [-8.0, 0.0, 1.0]);
    assert_eq!(plane[128].xyz, [0.0, 0.0, 1.0]);
    assert_eq!(plane[256].xyz, [-8.0, 0.0625, 1.0]);
    let step = cloud(CloudShape::Step, 256, FIXTURE_SEED).unwrap();
    assert_eq!(step[127].xyz[2], 1.0);
    assert_eq!(step[128].xyz[2], 2.0);
    let wall = cloud(CloudShape::CurvedWall, 129, FIXTURE_SEED).unwrap();
    assert_eq!(wall[0].xyz, [-8.0, 4.0, 0.0]);
    assert_eq!(wall[128].xyz, [0.0, 0.0, 0.0]);
}

#[test]
fn noisy_scan_validity_is_not_encoded_as_nan_or_zero_distance() {
    let samples = cloud(CloudShape::NoisyScan, 128, FIXTURE_SEED).unwrap();
    for (i, point) in samples.iter().enumerate() {
        assert_eq!(point.valid, i % 11 != 0);
        if !point.valid {
            assert_eq!(point.xyz, [0.0; 3]);
        } else {
            assert!((point.xyz[2] - 1.0).abs() <= 0.015625);
        }
    }
    assert_ne!(
        samples,
        cloud(CloudShape::NoisyScan, 128, FIXTURE_SEED + 1).unwrap()
    );
}

#[test]
fn fixture_allocation_limits_and_empty_manifests_are_explicit() {
    assert!(cloud(CloudShape::Plane, MAX_FIXTURE_POINTS + 1, 1).is_err());
    let empty = cloud_manifest(CloudShape::Plane, FIXTURE_SEED, &[]);
    assert_eq!(empty.count, 0);
    assert_eq!(empty.valid_count, 0);
    assert!(empty.queries.is_empty());
    assert_eq!(empty.checksum_fnv1a64, "cbf29ce484222325");
}

#[test]
fn occupancy_fixture_has_mixed_depths_states_and_negative_tiles_without_overlap() {
    let fixture = occupancy();
    assert_eq!(fixture.leaves.len(), 16);
    assert_eq!(
        fixture.root_span_m,
        fixture.leaf_resolution_m * 2f64.powi(fixture.max_depth.into())
    );
    let negative = &fixture.leaves[0];
    assert_eq!(negative.tile, [-1, 0, 0]);
    assert_eq!(negative.min[0] + negative.size_m, 0.0);
    assert!(fixture.leaves.iter().any(|leaf| leaf.probability.is_none()));
    assert!(
        fixture
            .leaves
            .iter()
            .any(|leaf| leaf.probability == Some(0.5))
    );
    assert!(
        fixture
            .leaves
            .iter()
            .any(|leaf| leaf.probability == Some(0.1))
    );
    for (i, a) in fixture.leaves.iter().enumerate() {
        assert_eq!(a.size_m, fixture.root_span_m / 2f64.powi(a.depth.into()));
        for b in &fixture.leaves[i + 1..] {
            let overlaps = (0..3).all(|axis| {
                a.min[axis] < b.min[axis] + b.size_m && b.min[axis] < a.min[axis] + a.size_m
            });
            assert!(!overlaps, "{a:?} overlaps {b:?}");
        }
    }
    let volume: f64 = fixture.leaves.iter().map(|leaf| leaf.size_m.powi(3)).sum();
    assert_eq!(volume, 2.0 * 16f64.powi(3));
}

#[test]
fn acceptance_profiles_keep_subbudgets_inside_total_limits() {
    let profiles: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/draw_data/profiles.json"
    ))
    .unwrap();
    assert_eq!(
        profiles["schema"],
        "gearbox.draw.data.acceptance_profiles.v1"
    );
    for name in ["reference", "low-memory"] {
        let p = &profiles[name];
        let bytes = |key: &str| p[key].as_u64().unwrap();
        assert!(bytes("writer_incomplete_bytes") <= bytes("incomplete_bytes"));
        assert!(bytes("incomplete_bytes") <= bytes("cpu_bytes"));
        assert!(bytes("writer_staged_bytes") <= bytes("cpu_bytes"));
        assert!(bytes("upload_bytes_per_frame") <= bytes("gpu_bytes"));
        assert_eq!(bytes("page_points"), 65_536);
        assert_eq!(bytes("lease_timeout_ms"), 10_000);
        assert_eq!(bytes("warmup_seconds"), 10);
        assert_eq!(bytes("measurement_seconds"), 60);
    }
    let reference = gearbox_api::wire::draw_data::DataLimits::reference();
    assert_eq!(profiles["reference"]["cpu_bytes"], reference.cpu_bytes);
    assert_eq!(profiles["reference"]["gpu_bytes"], reference.gpu_bytes);
    assert_eq!(profiles["reference"]["visible_points"], u64::from(reference.visible_points));
}

#[test]
fn cloud_and_occupancy_manifests_match_checked_in_oracles() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/draw_data/reference_1024");
    for shape in CLOUD_SHAPES {
        let samples = cloud(shape, 1024, FIXTURE_SEED).unwrap();
        let manifest = cloud_manifest(shape, FIXTURE_SEED, &samples);
        let name = serde_json::to_value(shape)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        let expected = std::fs::read(directory.join(format!("{name}.json"))).unwrap();
        assert_eq!(
            serde_json::to_vec_pretty(&manifest).unwrap(),
            expected,
            "{name}"
        );
    }
    let expected = std::fs::read(directory.join("occupancy.json")).unwrap();
    assert_eq!(serde_json::to_vec_pretty(&occupancy()).unwrap(), expected);
}
