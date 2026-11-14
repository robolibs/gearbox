use super::*;

pub const TITLES: [&str; 4] = [
    "01 / NANOSECOND AGE",
    "02 / U64 AGE CYCLES",
    "03 / SUBNORMAL AGE",
    "04 / FUTURE TIES",
];
pub const DETAILS: [&str; 4] = [
    "953.674 NS / 2^30 S ORIGIN",
    "EXACT OKABE CYCLES",
    "ZERO VS ONE NS FUTURE",
    "SIGNED AGE / HALF TIES",
];

pub fn schema() -> CloudSchema {
    let mut schema = super::schema();
    schema.attributes.push(Attribute {
        name: "acquisition".into(),
        scalar: ScalarType::U64,
        components: 1,
        semantic: AttributeSemantic::AcquisitionOffset,
        units: "ns".into(),
        normalization: Normalization::None,
        nullable: false,
    });
    schema
}

fn now(index: usize) -> u64 {
    match index {
        0 => (1u64 << 30) * 1_000_000_000 + 1024,
        1 => u64::MAX,
        2 => 0,
        _ => 500_000_000,
    }
}

pub fn stream() -> StreamMode {
    StreamMode::Accumulate {
        max_scans: 33,
        max_age_s: 1e12,
        max_bytes: 1 << 20,
        age_clock: AgeClock::Simulation,
    }
}

pub fn presentation(index: usize) -> Presentation {
    let (map, range) = match index {
        0 => (
            colormap::VIRIDIS,
            [(1u64 << 30) as f64, (1u64 << 30) as f64 + 2.0f64.powi(-20)],
        ),
        1 => (colormap::OKABE, [0.0, 1.0]),
        2 => (colormap::TURBO, [-f64::from_bits(1), f64::from_bits(1)]),
        _ => (colormap::OKABE, [-16.0, 1.0]),
    };
    Presentation {
        color: ColorMode::Age {
            clock: AgeClock::Source,
            colormap: map,
            range,
        },
        size: PointSize::Metres { radius: 0.021 },
        glow: 0.22,
        ..Default::default()
    }
}

fn offset(index: usize, band: usize, lane: usize) -> u64 {
    match index {
        0 => band as u64 * 64,
        2 => lane as u64,
        _ => band as u64 * 1_000_000_000,
    }
}

pub fn page(index: usize, band: usize, lane: usize) -> CloudPage {
    let mut page = clocks::page(band, lane);
    page.columns.push(ColumnData {
        values: (0..page.count)
            .flat_map(|_| offset(index, band, lane).to_le_bytes())
            .collect(),
        validity: vec![],
    });
    page
}

pub fn pages(schema: &CloudSchema, index: usize) -> Result<Vec<(PageDescriptor, Vec<u8>)>> {
    (0..32)
        .map(|i| {
            let page = page(index, i % 16, i / 16);
            let bytes = page.encode(schema)?;
            Ok((
                PageDescriptor {
                    address: PageAddress {
                        scan_key: i as u64 + 1,
                        page_id: 0,
                    },
                    encoding: PageEncoding::CloudColumnsV1,
                    items: page.count,
                    byte_len: bytes.len() as u64,
                    crc32: page_crc32(&bytes),
                    origin_m: [0.0; 3],
                },
                bytes,
            ))
        })
        .collect()
}

pub fn scans(index: usize, datum: &DatumSnapshot) -> Vec<ScanDescriptor> {
    let mut scans = clocks::scans(index, datum);
    let mut reference = scans[0].clone();
    reference.scan_key = 33;
    reference.source_count = 0;
    scans.push(reference);
    for (i, scan) in scans.iter_mut().enumerate() {
        scan.source = Some("synthetic/age-precision".into());
        let stamp = scan.frame.capture.as_mut().unwrap();
        stamp.source_clock = Some("precision-clock".into());
        stamp.source_time_ns = Some(if i == 32 { now(index) } else { 0 });
        scan.frame.capture_transform.as_mut().unwrap().stamp = stamp.clone();
    }
    scans
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precision_scene_preserves_offsets_and_uses_an_explicit_empty_reference_scan() {
        let schema = schema();
        let datum = DatumSnapshot {
            origin_ecef_m: [6_378_137.0, 0.0, 0.0],
            axes_to_ecef_wxyz: [1.0, 0.0, 0.0, 0.0],
        };
        for index in 0..4 {
            let scans = scans(index, &datum);
            let pages = pages(&schema, index).unwrap();
            let presentation = presentation(index);
            presentation.validate(Some(&schema), None, true).unwrap();
            let mut descriptor = DataDescriptor {
                revision: SCHEMA_REVISION,
                key: DataKey::new("precision-test", "cloud").unwrap(),
                shape: DataShape::Cloud {
                    schema: schema.clone(),
                },
                stream: stream(),
                lifetime_s: None,
                presentation,
            };
            descriptor.validate().unwrap();
            descriptor.validate_scans(&scans).unwrap();
            descriptor.stream = StreamMode::RetainedMap;
            assert!(descriptor.validate_scans(&scans).is_err());
            assert_eq!(scans.len(), 33);
            assert_eq!(scans[32].source_count, 0);
            assert_eq!(
                scans[32].frame.capture.as_ref().unwrap().source_time_ns,
                Some(now(index))
            );
            assert_eq!(
                pages.iter().map(|(page, _)| page.items).sum::<u32>(),
                clocks::POINTS
            );
            for (i, scan) in scans.iter().enumerate() {
                scan.validate().unwrap();
                if i < 32 {
                    assert_eq!(pages[i].0.address.scan_key, scan.scan_key);
                    scan.frame
                        .capture
                        .as_ref()
                        .unwrap()
                        .source_sample_ns(offset(index, i % 16, i / 16))
                        .unwrap();
                    let page = page(index, i % 16, i / 16);
                    assert_eq!(
                        u64::from_le_bytes(page.columns[5].values[..8].try_into().unwrap()),
                        offset(index, i % 16, i / 16)
                    );
                }
            }
        }
    }
}
