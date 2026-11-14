use gearbox_api::wire::{colormap, draw_data::*};

#[path = "support/draw_data_contracts.rs"]
mod contracts;
use contracts::*;

#[test]
fn optional_clip_bounds_preserve_old_json_and_validate_public_style_patches() {
    let old = cloud();
    let old_json = old.to_json().unwrap();
    assert!(!String::from_utf8_lossy(&old_json).contains("\"clip\""));
    let mut clipped = old.clone();
    clipped.presentation.clip = Some(ClipBounds {
        min_m: [-1.0; 3],
        max_m: [1.0; 3],
        ..Default::default()
    });
    assert_eq!(
        DataDescriptor::from_json(&clipped.to_json().unwrap()).unwrap(),
        clipped
    );
    let request = DataRequest {
        host_epoch: 1,
        lease: 1,
        ..DataRequest::new(data_op::PATCH_PRESENTATION)
    }
    .with_key(&clipped.key)
    .unwrap()
    .with_metadata(&clipped.presentation)
    .unwrap();
    assert!(matches!(
        request.operation().unwrap(),
        DataOperation::PatchPresentation(_)
    ));
    clipped.presentation.clip.as_mut().unwrap().max_m[0] = -1.0;
    assert!(clipped.validate().is_err());
    clipped.presentation.clip = None;
    assert_eq!(clipped.to_json().unwrap(), old_json);
}

#[test]
fn dataset_and_scan_json_are_independent_bounded_roundtrips() {
    for descriptor in [cloud(), octree()] {
        let json = descriptor.to_json().unwrap();
        assert!(json.len() <= MAX_DESCRIPTOR_BYTES);
        assert_eq!(DataDescriptor::from_json(&json).unwrap(), descriptor);
        let mut value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        value["unexpected"] = true.into();
        assert!(DataDescriptor::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    let scan = scan(1);
    assert_eq!(
        ScanDescriptor::from_json(&scan.to_json().unwrap()).unwrap(),
        scan
    );
    assert!(DataDescriptor::from_json(&vec![b' '; MAX_DESCRIPTOR_BYTES + 1]).is_err());
    assert!(ScanDescriptor::from_json(&vec![b' '; MAX_SCAN_BYTES + 1]).is_err());
    let mut unknown = scan.clone();
    unknown.revision += 1;
    assert!(unknown.to_json().is_err());
}

#[test]
fn keys_are_canonical_without_collapsing_interior_namespaces() {
    assert_eq!(DataKey::new("/a//b/", "id").unwrap().namespace, "a//b");
    assert!(DataKey::new("", "id").is_ok());
    for (ns, id) in [(" a", "b"), ("a", ""), ("a", "b\n"), ("a", " b")] {
        assert!(DataKey::new(ns, id).is_err());
    }
    assert!(DataKey::new(&"é".repeat(128), "id").is_err());
    assert!(DataKey::new("a", &"b".repeat(128)).is_err());
    assert!(
        DataKey {
            namespace: "/a".into(),
            id: "b".into()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn organization_counts_source_observations_not_rendered_points() {
    assert!(
        Organization {
            width: 2,
            height: 2
        }
        .validate(4)
        .is_ok()
    );
    assert!(
        Organization {
            width: 2,
            height: 2
        }
        .validate(3)
        .is_err()
    );
    assert!(
        Organization {
            width: 0,
            height: 0
        }
        .validate(0)
        .is_ok()
    );
    assert!(
        Organization {
            width: 0,
            height: 1
        }
        .validate(0)
        .is_err()
    );
    assert!(
        Organization {
            width: u32::MAX,
            height: u32::MAX
        }
        .validate(u64::from(u32::MAX).pow(2))
        .is_ok()
    );
}

#[test]
fn accumulated_scans_keep_distinct_capture_poses_times_and_organization() {
    let a = scan(1);
    let mut b = scan(2);
    b.organization = Some(Organization {
        width: 4,
        height: 1,
    });
    let before = a.to_json().unwrap();
    let scans = vec![a.clone(), b.clone()];
    cloud().validate_scans(&scans).unwrap();
    assert_eq!(scans[0].to_json().unwrap(), before);
    assert_eq!(
        a.frame
            .target_point([0.0; 3], [1.0, 2.0, 3.0], None)
            .unwrap(),
        [11.0, 2.0, 3.0]
    );
    assert_eq!(
        b.frame
            .target_point([0.0; 3], [1.0, 2.0, 3.0], None)
            .unwrap(),
        [21.0, 2.0, 3.0]
    );
    assert_eq!(a.frame.capture.as_ref().unwrap().simulation_time_s, 1.0);
    assert_eq!(b.frame.capture.as_ref().unwrap().simulation_time_s, 2.0);
    assert_ne!(a.organization, b.organization);
}

#[test]
fn scan_keys_and_live_capture_rules_follow_stream_mode() {
    let mut descriptor = cloud();
    assert!(descriptor.validate_scans(&[scan(1), scan(1)]).is_err());
    assert!(descriptor.validate_scan(&scan(0)).is_err());
    assert!(
        descriptor
            .validate_scans(&(1..=5).map(scan).collect::<Vec<_>>())
            .is_err()
    );
    let mut live = scan(1);
    live.frame.placement = Placement::Live;
    live.frame.capture_transform = None;
    assert!(descriptor.validate_scan(&live).is_err());
    descriptor.stream = StreamMode::Latest;
    descriptor.validate_scan(&live).unwrap();
    descriptor.validate_scan(&scan(2)).unwrap();
    assert!(descriptor.validate_scan(&scan(0)).is_err());
    assert!(descriptor.validate_scans(&[scan(1), scan(2)]).is_err());
    live.frame.capture = None;
    assert!(descriptor.validate_scan(&live).is_err());
    descriptor.stream = StreamMode::RetainedMap;
    live.scan_key = 0;
    descriptor.validate_scan(&live).unwrap();
    assert!(descriptor.validate_scan(&scan(1)).is_err());
    assert!(octree().validate_scan(&live).is_err());
}

#[test]
fn source_age_accumulation_requires_one_clock_and_scene_epoch() {
    let mut descriptor = cloud();
    descriptor.stream = StreamMode::Accumulate {
        max_scans: 4,
        max_age_s: 3.0,
        max_bytes: 1024,
        age_clock: AgeClock::Source,
    };
    let a = scan(1);
    let mut b = scan(2);
    descriptor.validate_scans(&[a.clone(), b.clone()]).unwrap();
    b.frame.capture.as_mut().unwrap().source_clock = Some("other".into());
    b.frame.capture_transform.as_mut().unwrap().stamp = b.frame.capture.clone().unwrap();
    assert!(descriptor.validate_scans(&[a.clone(), b]).is_err());
    let mut b = scan(2);
    b.frame.capture.as_mut().unwrap().scene_epoch = 2;
    b.frame.capture_transform.as_mut().unwrap().stamp = b.frame.capture.clone().unwrap();
    assert!(descriptor.validate_scans(&[a, b]).is_err());
    let mut b = scan(2);
    b.frame.capture.as_mut().unwrap().source_time_ns = None;
    b.frame.capture.as_mut().unwrap().source_clock = None;
    b.frame.capture_transform.as_mut().unwrap().stamp = b.frame.capture.clone().unwrap();
    assert!(descriptor.validate_scan(&b).is_err());
}

#[test]
fn descriptor_limits_fail_closed() {
    let mut descriptor = cloud();
    for ttl in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        descriptor.lifetime_s = Some(ttl);
        assert!(descriptor.validate().is_err());
    }
    descriptor.lifetime_s = None;
    for (max_scans, max_age_s, max_bytes) in [
        (0, 1.0, 1),
        (MAX_SCANS + 1, 1.0, 1),
        (1, 0.0, 1),
        (1, f64::INFINITY, 1),
        (1, 1.0, 0),
    ] {
        descriptor.stream = StreamMode::Accumulate {
            max_scans,
            max_age_s,
            max_bytes,
            age_clock: AgeClock::Simulation,
        };
        assert!(descriptor.validate().is_err());
    }
    let mut tree = octree();
    tree.stream = cloud().stream;
    assert!(tree.validate().is_err());
    descriptor = cloud();
    descriptor.revision += 1;
    assert!(descriptor.validate().is_err());
}

#[test]
fn capture_requires_matching_stamp_source_and_anchored_target() {
    let good = scan(1).frame;
    for change in 0..6 {
        let mut frame = good.clone();
        let transform = frame.capture_transform.as_mut().unwrap();
        match change {
            0 => transform.source_frame = "wrong/link".into(),
            1 => transform.stamp.sample += 1,
            2 => transform.target_frame = "robot/lidar".into(),
            3 => transform.datum = None,
            4 => transform.datum.as_mut().unwrap().origin_ecef_m[0] = f64::NAN,
            _ => transform.revision += 1,
        }
        assert!(frame.validate().is_err(), "case {change}");
    }
    let mut live = good.clone();
    live.placement = Placement::Live;
    assert!(live.validate().is_err());
    live.capture_transform = None;
    live.validate().unwrap();
    assert!(live.target_point([0.0; 3], [0.0; 3], None).is_err());
    assert!(live.captured_ecef_point([0.0; 3], [0.0; 3]).is_err());
}

#[test]
fn rigid_transform_order_handles_origins_rotations_and_captured_datums() {
    let mut frame = scan(1).frame;
    let q = [
        std::f64::consts::FRAC_1_SQRT_2,
        0.0,
        0.0,
        std::f64::consts::FRAC_1_SQRT_2,
    ];
    frame.origin_m = [2.0, 0.0, 0.0];
    frame.pose = RigidPose {
        translation_m: [3.0, 0.0, 0.0],
        rotation_wxyz: q,
    };
    frame.capture_transform.as_mut().unwrap().pose.rotation_wxyz = q;
    let source = frame
        .source_point([1.0, 0.0, 0.0], [1.0, 0.0, 0.0])
        .unwrap();
    close(source, [3.0, 4.0, 0.0], 1e-12);
    close(
        frame
            .target_point(
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                Some(&RigidPose {
                    translation_m: [999.0; 3],
                    ..Default::default()
                }),
            )
            .unwrap(),
        [6.0, 3.0, 0.0],
        1e-12,
    );
    close(
        frame
            .captured_ecef_point([1.0, 0.0, 0.0], [1.0, 0.0, 0.0])
            .unwrap(),
        [6_378_143.0, 3.0, 0.0],
        1e-9,
    );
    close(
        frame.pose.transform_vector([1.0, 0.0, 0.0]).unwrap(),
        [0.0, 1.0, 0.0],
        1e-12,
    );
    let mut invalid = frame.clone();
    invalid.pose.rotation_wxyz = [0.0; 4];
    assert!(invalid.validate().is_err());
    invalid = frame;
    invalid.origin_m[0] = f64::MAX;
    assert!(invalid.source_point([f64::MAX; 3], [1.0; 3]).is_err());
}

fn close(actual: [f64; 3], expected: [f64; 3], tolerance: f64) {
    for i in 0..3 {
        assert!(
            (actual[i] - expected[i]).abs() <= tolerance,
            "{actual:?} != {expected:?}"
        );
    }
}

#[test]
fn source_timestamps_preserve_u64_precision_and_reject_overflow() {
    let mut stamp = scan(1).frame.capture.unwrap();
    stamp.source_time_ns = Some(u64::MAX - 1);
    assert_eq!(stamp.source_sample_ns(1).unwrap(), u64::MAX);
    assert!(stamp.source_sample_ns(2).is_err());
    stamp.source_clock = None;
    assert!(stamp.validate().is_err());
    stamp.source_time_ns = None;
    stamp.validate().unwrap();
    assert!(stamp.source_sample_ns(0).is_err());
    stamp.scene_epoch = 0;
    assert!(stamp.validate().is_err());
}

#[test]
fn presentation_preserves_explicit_colors_and_independent_blur() {
    let mut descriptor = cloud();
    descriptor.presentation.color = ColorMode::Constant {
        rgba: [255, 0, 65, 83],
    };
    let original = descriptor.presentation.color.clone();
    for blur in [0.0, 0.5, 1.0] {
        descriptor.presentation.blur = blur;
        descriptor.validate().unwrap();
        assert_eq!(descriptor.presentation.color, original);
        assert_eq!(descriptor.presentation.opacity, 1.0);
        assert_eq!(descriptor.presentation.glow, 0.0);
    }
    for blur in [-0.1, 1.1, f32::NAN] {
        descriptor.presentation.blur = blur;
        assert!(descriptor.validate().is_err());
    }
    descriptor = cloud();
    descriptor.presentation.radius_attribute = Some("radius".into());
    descriptor.validate().unwrap();
    descriptor.presentation.radius_attribute = Some("intensity".into());
    assert!(descriptor.validate().is_err());
}

#[test]
fn color_modes_validate_semantics_not_only_attribute_names() {
    let mut descriptor = cloud();
    for color in [
        ColorMode::Attribute {
            name: "rgba".into(),
            space: ColorSpace::Srgb,
        },
        ColorMode::Scalar {
            name: "intensity".into(),
            colormap: colormap::VIRIDIS,
            range: [0.0, 1.0],
            missing: [0; 4],
        },
        ColorMode::Category {
            name: "class".into(),
            entries: vec![PaletteEntry {
                value: u32::MAX,
                rgba: [1, 2, 3, 4],
            }],
            missing: [0; 4],
        },
        ColorMode::Axis {
            axis: 2,
            colormap: colormap::VIRIDIS,
            range: [0.0, 0.0],
        },
        ColorMode::Range {
            origin_m: [0.0; 3],
            colormap: colormap::VIRIDIS,
            range: [0.0, 10.0],
        },
        ColorMode::Age {
            clock: AgeClock::Source,
            colormap: colormap::VIRIDIS,
            range: [0.0, 10.0],
        },
    ] {
        descriptor.presentation.color = color;
        descriptor.validate_scan(&scan(1)).unwrap();
    }
    for color in [
        ColorMode::Attribute {
            name: "class".into(),
            space: ColorSpace::Srgb,
        },
        ColorMode::Scalar {
            name: "position".into(),
            colormap: colormap::VIRIDIS,
            range: [0.0, 1.0],
            missing: [0; 4],
        },
        ColorMode::Scalar {
            name: "intensity".into(),
            colormap: u32::MAX,
            range: [0.0, 1.0],
            missing: [0; 4],
        },
        ColorMode::Axis {
            axis: 3,
            colormap: colormap::VIRIDIS,
            range: [0.0, 1.0],
        },
        ColorMode::Category {
            name: "class".into(),
            entries: vec![
                PaletteEntry {
                    value: 1,
                    rgba: [1; 4]
                };
                2
            ],
            missing: [0; 4],
        },
        ColorMode::OctreeDepth {
            colormap: colormap::VIRIDIS,
        },
    ] {
        descriptor.presentation.color = color;
        assert!(descriptor.validate().is_err());
    }
}

#[test]
fn age_color_requires_capture_even_for_static_clouds() {
    let mut descriptor = cloud();
    descriptor.stream = StreamMode::RetainedMap;
    descriptor.presentation.color = ColorMode::Age {
        clock: AgeClock::Simulation,
        colormap: colormap::VIRIDIS,
        range: [0.0, 1.0],
    };
    let mut input = scan(0);
    input.frame = DataFrame::default();
    assert!(descriptor.validate_scan(&input).is_err());
    input.frame.capture = scan(1).frame.capture;
    descriptor.validate_scan(&input).unwrap();
}

#[test]
fn unknown_space_requires_explicit_bounded_region() {
    let mut descriptor = octree();
    descriptor
        .presentation
        .octree
        .as_mut()
        .unwrap()
        .show_unknown = true;
    assert!(descriptor.validate().is_err());
    descriptor.presentation.octree.as_mut().unwrap().roi = Some(ClipBounds {
        min_m: [-1.0; 3],
        max_m: [1.0; 3],
        ..Default::default()
    });
    descriptor.validate().unwrap();
    descriptor
        .presentation
        .octree
        .as_mut()
        .unwrap()
        .unknown_cell_budget = 1_000_001;
    assert!(descriptor.validate().is_err());
    descriptor = octree();
    descriptor.presentation.octree.as_mut().unwrap().depth_limit = Some(13);
    assert!(descriptor.validate().is_err());
    descriptor.presentation.octree = None;
    assert!(descriptor.validate().is_err());
}

#[test]
fn clip_bounds_reject_nonfinite_extents_even_with_finite_corners() {
    assert!(
        ClipBounds {
            min_m: [-f64::MAX; 3],
            max_m: [f64::MAX; 3],
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        ClipBounds {
            min_m: [0.0; 3],
            max_m: [0.0; 3],
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        ClipBounds {
            min_m: [-1.0; 3],
            max_m: [1.0; 3],
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn turned_and_inverted_clips_roundtrip_and_plain_ones_keep_their_json() {
    let plain = ClipBounds {
        min_m: [0.0; 3],
        max_m: [1.0; 3],
        ..Default::default()
    };
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("rotation") && !json.contains("invert"), "{json}");
    let turned = ClipBounds {
        rotation_wxyz: Some([0.5f64.sqrt(), 0.0, 0.0, 0.5f64.sqrt()]),
        invert: true,
        ..plain
    };
    turned.validate().unwrap();
    let back: ClipBounds = serde_json::from_str(&serde_json::to_string(&turned).unwrap()).unwrap();
    assert_eq!(back, turned);
    let skewed = ClipBounds {
        rotation_wxyz: Some([1.0, 1.0, 0.0, 0.0]),
        ..plain
    };
    assert!(skewed.validate().is_err());
    let mut display = OctreeDisplay {
        show_unknown: true,
        roi: Some(turned),
        unknown_cell_budget: 10,
        ..OctreeDisplay::default()
    };
    let config = OctreeConfig { leaf_resolution_m: 0.1, max_depth: 8, free_max: 0.3, occupied_min: 0.7 };
    assert!(display.validate(&config).is_err(), "an ROI stays axis-aligned");
    display.roi = Some(plain);
    display.validate(&config).unwrap();
}
