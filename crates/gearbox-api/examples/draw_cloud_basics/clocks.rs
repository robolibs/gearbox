use super::*;

pub const POINTS: u32 = 3072;

fn earth_datum(latitude: f64, longitude: f64) -> DatumSnapshot {
    let origin = concord::earth::to_ecf(concord::earth::Wgs::new(latitude, longitude, 0.0));
    let (sx, cx) = ((std::f64::consts::FRAC_PI_2 - latitude.to_radians()) * 0.5).sin_cos();
    let (sz, cz) = ((longitude.to_radians() + std::f64::consts::FRAC_PI_2) * 0.5).sin_cos();
    DatumSnapshot {
        origin_ecef_m: [origin.x, origin.y, origin.z],
        axes_to_ecef_wxyz: [cz * cx, cz * sx, sz * sx, sz * cx],
    }
}

pub fn datum(client: &Client) -> Result<DatumSnapshot> {
    let home = client
        .info()?
        .props()
        .get("home")
        .ok_or("preview requires the host home datum")?;
    let values: Vec<f64> = home
        .split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    if values.len() != 2
        || values.iter().any(|v| !v.is_finite())
        || values[0].abs() > 90.0
        || values[1].abs() > 180.0
    {
        return Err("invalid preview home latitude/longitude".into());
    }
    Ok(earth_datum(values[0], values[1]))
}

pub fn presentation(index: usize) -> Presentation {
    Presentation {
        color: ColorMode::Age {
            clock: AgeClock::Source,
            colormap: [
                colormap::VIRIDIS,
                colormap::TURBO,
                colormap::OKABE,
                colormap::GREY,
            ][index],
            range: [0.0, 15.0],
        },
        size: PointSize::Metres { radius: 0.021 },
        glow: 0.22,
        ..Default::default()
    }
}

pub fn stream() -> StreamMode {
    StreamMode::Accumulate {
        max_scans: 32,
        max_age_s: 1e12,
        max_bytes: 1 << 20,
        age_clock: AgeClock::Simulation,
    }
}

pub fn page(band: usize, lane: usize) -> CloudPage {
    let mut columns = vec![
        ColumnData {
            values: Vec::new(),
            validity: Vec::new()
        };
        5
    ];
    for z in 0..16 {
        for x in 0..6 {
            let xyz = [
                -2.2 + band as f64 * 4.4 / 16.0 + x as f64 * 0.035,
                0.0,
                1.0 + lane as f64 * 1.15 + z as f64 * 0.065,
            ];
            columns[0]
                .values
                .extend(xyz.into_iter().flat_map(f64::to_le_bytes));
            columns[1]
                .values
                .extend((u64::MAX - (z * 6 + x) as u64).to_le_bytes());
            columns[2].values.extend([255; 4]);
            columns[3].values.extend((band as f32).to_le_bytes());
            columns[4].values.extend((lane as u32).to_le_bytes());
        }
    }
    CloudPage { count: 96, columns }
}

pub fn pages(schema: &CloudSchema) -> Result<Vec<(PageDescriptor, Vec<u8>)>> {
    let mut result = Vec::new();
    for lane in 0..2 {
        for band in 0..16 {
            let page = page(band, lane);
            let bytes = page.encode(schema)?;
            result.push((
                PageDescriptor {
                    address: PageAddress {
                        scan_key: (lane * 16 + band + 1) as u64,
                        page_id: 0,
                    },
                    encoding: PageEncoding::CloudColumnsV1,
                    items: page.count,
                    byte_len: bytes.len() as u64,
                    crc32: page_crc32(&bytes),
                    origin_m: [0.0; 3],
                },
                bytes,
            ));
        }
    }
    Ok(result)
}

pub fn scans(index: usize, datum: &DatumSnapshot) -> Vec<ScanDescriptor> {
    let [x, y] = super::centers()[index];
    (0..32)
        .map(|i| {
            let band = i % 16;
            let lane = i / 16;
            let stamp = CaptureStamp {
                simulation_time_s: 0.0,
                source_time_ns: Some(if lane == 0 {
                    (15 - band) * 1_000_000_000
                } else {
                    u64::MAX - band * 1_000_000_000
                }),
                source_clock: Some(if lane == 0 { "sensor-a" } else { "sensor-b" }.into()),
                sample: i,
                scene_epoch: 1,
            };
            ScanDescriptor {
                revision: SCHEMA_REVISION,
                scan_key: i + 1,
                source: Some("synthetic/mixed-source-clocks".into()),
                frame: DataFrame {
                    origin_m: [x, y, 0.0],
                    placement: Placement::Snapshot,
                    capture: Some(stamp.clone()),
                    capture_transform: Some(CaptureTransform {
                        revision: SCHEMA_REVISION,
                        source_frame: "world".into(),
                        target_frame: "world".into(),
                        stamp,
                        pose: RigidPose::default(),
                        datum: Some(datum.clone()),
                    }),
                    ..Default::default()
                },
                source_count: 96,
                organization: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_clock_rows_have_identical_ages_but_unrelated_timestamp_origins() {
        let schema = super::super::schema();
        let pages = pages(&schema).unwrap();
        assert_eq!(
            pages.iter().map(|(page, _)| page.items).sum::<u32>(),
            POINTS
        );
        for index in 0..4 {
            let descriptor = DataDescriptor {
                revision: SCHEMA_REVISION,
                key: DataKey::new(NS, "test").unwrap(),
                shape: DataShape::Cloud {
                    schema: schema.clone(),
                },
                stream: stream(),
                lifetime_s: None,
                presentation: presentation(index),
            };
            let scans = scans(index, &earth_datum(52.370216, 4.895168));
            descriptor.validate_scans(&scans).unwrap();
            for band in 0..16 {
                for lane in 0..2 {
                    let stamp = scans[lane * 16 + band].frame.capture.as_ref().unwrap();
                    let newest = scans[lane * 16].frame.capture.as_ref().unwrap();
                    assert_eq!(
                        newest.source_time_ns.unwrap() - stamp.source_time_ns.unwrap(),
                        band as u64 * 1_000_000_000
                    );
                    assert_eq!(
                        pages[lane * 16 + band].0.address.scan_key,
                        scans[lane * 16 + band].scan_key
                    );
                }
            }
        }
    }

    #[test]
    fn earth_preview_datum_maps_enu_axes_at_the_equator() {
        let datum = earth_datum(0.0, 0.0);
        assert_eq!(datum.origin_ecef_m, [6_378_137.0, 0.0, 0.0]);
        let pose = RigidPose {
            translation_m: datum.origin_ecef_m,
            rotation_wxyz: datum.axes_to_ecef_wxyz,
        };
        for (input, expected) in [
            ([1.0, 0.0, 0.0], [6_378_137.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [6_378_137.0, 0.0, 1.0]),
            ([0.0, 0.0, 1.0], [6_378_138.0, 0.0, 0.0]),
        ] {
            let actual = pose.transform_point(input).unwrap();
            assert!((0..3).all(|axis| (actual[axis] - expected[axis]).abs() < 1e-8));
        }
    }
}
