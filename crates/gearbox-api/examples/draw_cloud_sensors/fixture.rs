use super::*;

pub const POINTS: u32 = 49 * 33;

pub struct Fixture {
    pub descriptor: DataDescriptor,
    pub scans: [ScanDescriptor; 1],
    pub page: PageDescriptor,
    pub bytes: Vec<u8>,
}

pub fn presentation(opacity: f32) -> Presentation {
    Presentation {
        color: ColorMode::Constant {
            rgba: [255, 0, 255, 255],
        },
        size: PointSize::Metres { radius: 0.14 },
        opacity,
        glow: 0.2,
        blur: 0.8,
        ..Default::default()
    }
}

pub fn make(frame: &str) -> Result<Fixture> {
    let schema = CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            Attribute {
                name: "position".into(),
                scalar: ScalarType::F32,
                components: 3,
                semantic: AttributeSemantic::Position,
                units: "m".into(),
                normalization: Normalization::None,
                nullable: false,
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
        ],
    };
    let mut values = Vec::new();
    for z in 0..33 {
        for y in 0..49 {
            values.extend(
                [2.0, y as f32 * 0.125 - 3.0, z as f32 * 0.125 - 2.0]
                    .into_iter()
                    .flat_map(f32::to_le_bytes),
            );
        }
    }
    let bytes = CloudPage {
        count: POINTS,
        columns: vec![
            ColumnData {
                values,
                validity: vec![],
            },
            ColumnData {
                values: (0..u64::from(POINTS)).flat_map(u64::to_le_bytes).collect(),
                validity: vec![],
            },
        ],
    }
    .encode(&schema)?;
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, "sensor-plane")?,
        shape: DataShape::Cloud { schema },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: presentation(1.0),
    };
    let scans = [ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: Some("native-sensor-isolation".into()),
        frame: DataFrame {
            frame: frame.into(),
            ..Default::default()
        },
        source_count: u64::from(POINTS),
        organization: None,
    }];
    descriptor.validate_scans(&scans)?;
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
        scans,
        page,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensor_plane_covers_forward_camera_cone() {
        let value = make("sensor_probe/camera_link").unwrap();
        let DataShape::Cloud { schema } = &value.descriptor.shape else {
            panic!()
        };
        let decoded = CloudPage::decode(schema, &value.bytes).unwrap();
        assert_eq!(decoded.count, POINTS);
        assert_eq!(value.page.crc32, page_crc32(&value.bytes));
        let positions: Vec<_> = decoded.columns[0]
            .values
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(&positions[..3], &[2.0, -3.0, -2.0]);
        assert_eq!(&positions[positions.len() - 3..], &[2.0, 3.0, 2.0]);
        assert!(
            positions
                .chunks_exact(3)
                .all(|p| p[0] == 2.0 && p.iter().all(|v| v.is_finite()))
        );
        assert_eq!(value.scans[0].frame.frame, "sensor_probe/camera_link");
        assert_eq!(value.scans[0].frame.placement, Placement::Live);
        assert!(2.0 * (30.0f64.to_radians()).tan() < 2.0);
    }

    #[test]
    fn both_opaque_and_transmitting_styles_keep_frost_requested() {
        for opacity in [1.0, 0.55] {
            let style = presentation(opacity);
            assert_eq!(style.opacity, opacity);
            assert!(style.blur > 0.0 && style.glow > 0.0);
            assert!(matches!(style.size, PointSize::Metres { radius } if radius > 0.125));
        }
    }
}
