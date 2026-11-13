use gearbox_api::wire::draw_data::*;

pub fn schema() -> CloudSchema {
    CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            Attribute {
                name: "position".into(),
                scalar: ScalarType::F64,
                components: 3,
                semantic: AttributeSemantic::Position,
                units: "m".into(),
                normalization: Normalization::None,
                nullable: true,
            },
            Attribute {
                name: "source_id".into(),
                scalar: ScalarType::U64,
                components: 1,
                semantic: AttributeSemantic::SourceId,
                units: "".into(),
                normalization: Normalization::None,
                nullable: false,
            },
            Attribute {
                name: "rgba".into(),
                scalar: ScalarType::U8,
                components: 4,
                semantic: AttributeSemantic::Color,
                units: "".into(),
                normalization: Normalization::Unorm,
                nullable: true,
            },
            Attribute {
                name: "intensity".into(),
                scalar: ScalarType::U16,
                components: 1,
                semantic: AttributeSemantic::Intensity,
                units: "".into(),
                normalization: Normalization::Unorm,
                nullable: true,
            },
            Attribute {
                name: "class".into(),
                scalar: ScalarType::U32,
                components: 1,
                semantic: AttributeSemantic::Category,
                units: "".into(),
                normalization: Normalization::None,
                nullable: true,
            },
            Attribute {
                name: "radius".into(),
                scalar: ScalarType::F32,
                components: 1,
                semantic: AttributeSemantic::Radius,
                units: "m".into(),
                normalization: Normalization::None,
                nullable: true,
            },
        ],
    }
}

pub fn cloud() -> DataDescriptor {
    DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new("mapping", "cloud").unwrap(),
        shape: DataShape::Cloud { schema: schema() },
        stream: StreamMode::Accumulate {
            max_scans: 4,
            max_age_s: 3.0,
            max_bytes: 1024 * 1024,
            age_clock: AgeClock::Simulation,
        },
        lifetime_s: None,
        presentation: Presentation::default(),
    }
}

pub fn scan(key: u64) -> ScanDescriptor {
    let stamp = CaptureStamp {
        simulation_time_s: key as f64,
        source_time_ns: Some(key * 1_000_000_000),
        source_clock: Some("lidar-clock".into()),
        sample: key,
        scene_epoch: 1,
    };
    let frame = "robot/lidar".to_string();
    ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: key,
        source: Some("robot/lidar".into()),
        frame: DataFrame {
            frame: frame.clone(),
            placement: Placement::Snapshot,
            capture: Some(stamp.clone()),
            capture_transform: Some(CaptureTransform {
                revision: 1,
                source_frame: frame,
                target_frame: "world".into(),
                stamp,
                pose: RigidPose {
                    translation_m: [10.0 * key as f64, 0.0, 0.0],
                    ..Default::default()
                },
                datum: Some(DatumSnapshot {
                    origin_ecef_m: [6_378_137.0, 0.0, 0.0],
                    axes_to_ecef_wxyz: [1.0, 0.0, 0.0, 0.0],
                }),
            }),
            ..Default::default()
        },
        source_count: 4,
        organization: Some(Organization {
            width: 2,
            height: 2,
        }),
    }
}

pub fn octree() -> DataDescriptor {
    DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new("mapping", "octree").unwrap(),
        shape: DataShape::Octree {
            config: OctreeConfig {
                leaf_resolution_m: 0.05,
                max_depth: 12,
                free_max: 0.3,
                occupied_min: 0.7,
            },
            frame: DataFrame::default(),
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation::for_octree(),
    }
}
