use super::*;

pub const POINTS: u32 = 27;
pub const IDS: [&str; 4] = ["world", "odom", "live", "snapshot"];
const COLORS: [[u8; 4]; 4] = [
    [255, 255, 0, 255],
    [0, 200, 255, 255],
    [0, 255, 0, 255],
    [255, 0, 255, 255],
];

pub struct Fixture {
    pub descriptor: DataDescriptor,
    pub scan: ScanDescriptor,
    pub page: PageDescriptor,
    pub bytes: Vec<u8>,
}

pub fn datum(latitude: f64, longitude: f64) -> DatumSnapshot {
    let origin = concord::earth::to_ecf(concord::earth::Wgs::new(latitude, longitude, 0.0));
    let (sx, cx) = ((std::f64::consts::FRAC_PI_2 - latitude.to_radians()) * 0.5).sin_cos();
    let (sz, cz) = ((longitude.to_radians() + std::f64::consts::FRAC_PI_2) * 0.5).sin_cos();
    DatumSnapshot {
        origin_ecef_m: [origin.x, origin.y, origin.z],
        axes_to_ecef_wxyz: [cz * cx, cz * sx, sz * sx, sz * cx],
    }
}

pub fn make(index: usize, machine: &str, datum: DatumSnapshot) -> Result<Fixture> {
    let schema = CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: [
            ("position", ScalarType::F32, 3, AttributeSemantic::Position),
            ("source_id", ScalarType::U64, 1, AttributeSemantic::SourceId),
        ]
        .into_iter()
        .map(|(name, scalar, components, semantic)| Attribute {
            name: name.into(),
            scalar,
            components,
            semantic,
            units: if semantic == AttributeSemantic::Position {
                "m"
            } else {
                ""
            }
            .into(),
            normalization: Normalization::None,
            nullable: false,
        })
        .collect(),
    };
    let mut positions = Vec::new();
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                positions.extend(
                    [x as f32 * 0.15, y as f32 * 0.15, z as f32 * 0.15]
                        .into_iter()
                        .flat_map(f32::to_le_bytes),
                );
            }
        }
    }
    let bytes = CloudPage {
        count: POINTS,
        columns: vec![
            ColumnData {
                values: positions,
                validity: vec![],
            },
            ColumnData {
                values: (0..u64::from(POINTS))
                    .flat_map(|i| (u64::MAX - i).to_le_bytes())
                    .collect(),
                validity: vec![],
            },
        ],
    }
    .encode(&schema)?;
    let mut frame = DataFrame {
        frame: match index {
            0 => "world".into(),
            1 => format!("{machine}/odom"),
            _ => machine.into(),
        },
        pose: RigidPose {
            translation_m: [
                [-3.0, -3.0, 3.0],
                [3.0, -3.0, 3.0],
                [3.0, 3.0, 3.0],
                [-3.0, 3.0, 3.0],
            ][index],
            ..Default::default()
        },
        ..Default::default()
    };
    if index == 3 {
        let stamp = CaptureStamp {
            simulation_time_s: 0.0,
            source_time_ns: Some(1_000_000_000),
            source_clock: Some("synthetic-frame-fixture".into()),
            sample: 1,
            scene_epoch: 1,
        };
        frame.placement = Placement::Snapshot;
        frame.capture = Some(stamp.clone());
        frame.capture_transform = Some(CaptureTransform {
            revision: 1,
            source_frame: machine.into(),
            target_frame: "world".into(),
            stamp,
            pose: RigidPose::default(),
            datum: Some(datum),
        });
    }
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, IDS[index])?,
        shape: DataShape::Cloud { schema },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Constant {
                rgba: COLORS[index],
            },
            size: PointSize::Metres { radius: 0.15 },
            glow: 0.2,
            blur: 0.2,
            ..Default::default()
        },
    };
    let scan = ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: Some("synthetic-frame-contract".into()),
        frame,
        source_count: u64::from(POINTS),
        organization: None,
    };
    descriptor.validate_scans(std::slice::from_ref(&scan))?;
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: POINTS,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    Ok(Fixture {
        descriptor,
        scan,
        page,
        bytes,
    })
}

pub fn replace_capture(frame: &mut DataFrame) -> Result<()> {
    let stamp = frame.capture.as_mut().ok_or("missing capture")?;
    stamp.sample += 1;
    stamp.source_time_ns = Some(2_000_000_000);
    stamp.simulation_time_s = 1.0;
    let transform = frame
        .capture_transform
        .as_mut()
        .ok_or("missing capture transform")?;
    transform.stamp = stamp.clone();
    let (s, c) = std::f64::consts::FRAC_PI_4.sin_cos();
    transform.pose = RigidPose {
        translation_m: [4.0, 0.0, 0.0],
        rotation_wxyz: [c, 0.0, 0.0, s],
    };
    frame.validate()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_frames_share_exact_source_records_but_not_placement() {
        let fixtures: Vec<_> = (0..4)
            .map(|i| make(i, "robot", datum(52.370216, 4.895168)).unwrap())
            .collect();
        for f in &fixtures {
            assert_eq!(f.bytes, fixtures[0].bytes);
            let DataShape::Cloud { schema } = &f.descriptor.shape else {
                panic!()
            };
            let cloud = CloudPage::decode(schema, &f.bytes).unwrap();
            assert_eq!(cloud.count, POINTS);
            for (i, bytes) in cloud.columns[1].values.chunks_exact(8).enumerate() {
                assert_eq!(
                    u64::from_le_bytes(bytes.try_into().unwrap()),
                    u64::MAX - i as u64
                );
            }
        }
        assert_eq!(fixtures[0].scan.frame.frame, "world");
        assert_eq!(fixtures[1].scan.frame.frame, "robot/odom");
        assert_eq!(fixtures[2].scan.frame.placement, Placement::Live);
        assert_eq!(fixtures[3].scan.frame.placement, Placement::Snapshot);
    }

    #[test]
    fn local_and_capture_deltas_keep_payload_and_exact_stamp() {
        let mut f = make(3, "robot", datum(52.370216, 4.895168)).unwrap();
        let bytes = f.bytes.clone();
        f.scan.frame.pose.translation_m[0] += 1.0;
        replace_capture(&mut f.scan.frame).unwrap();
        assert_eq!(bytes, f.bytes);
        let capture = f.scan.frame.capture_transform.as_ref().unwrap();
        assert_eq!(Some(&capture.stamp), f.scan.frame.capture.as_ref());
        let point = capture
            .pose
            .transform_point(f.scan.frame.source_point([0.0; 3], [0.0; 3]).unwrap())
            .unwrap();
        for (actual, expected) in point.into_iter().zip([1.0, -2.0, 3.0]) {
            assert!((actual - expected).abs() < 1e-12);
        }
    }
}
