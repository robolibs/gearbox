use super::*;
use gearbox_api::draw_data::survey;

pub const POINTS: u32 = 1_000_000;
#[allow(dead_code)]
pub const PAGE: u32 = survey::SURVEY_PAGE;

pub struct Fixture {
    pub descriptor: DataDescriptor,
    pub scans: Vec<ScanDescriptor>,
    pub pages: Vec<(PageDescriptor, Vec<u8>)>,
}

pub fn presentation() -> Presentation {
    Presentation {
        color: ColorMode::Attribute {
            name: "rgba".into(),
            space: ColorSpace::Srgb,
        },
        size: PointSize::Pixels { radius: 0.8 },
        glow: 0.22,
        ..Default::default()
    }
}

pub fn schema() -> CloudSchema {
    survey::survey_schema()
}

#[cfg(test)]
fn sample(index: u32, side: u32) -> ([f32; 3], [u8; 4]) {
    survey::survey_sample(index, side)
}

pub fn make(key: DataKey, empty: bool) -> Result<Fixture> {
    make_sized(key, if empty { 0 } else { POINTS })
}

pub fn make_sized(key: DataKey, points: u32) -> Result<Fixture> {
    require(
        points <= survey::SURVEY_MAX_POINTS,
        "survey fixture supports at most 5M source points",
    )?;
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key,
        shape: DataShape::Cloud { schema: schema() },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: presentation(),
    };
    let pages = (0..survey::survey_pages(points))
        .map(|page| survey::survey_page(points, page).ok_or("survey page encoding failed"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let scans = if points == 0 {
        vec![]
    } else {
        vec![survey::survey_scan(points)]
    };
    descriptor.validate_scans(&scans)?;
    Ok(Fixture {
        descriptor,
        scans,
        pages,
    })
}

pub fn manifest() -> serde_json::Value {
    serde_json::json!({"overview":{"focus":[0,0,3],"distance":58,"pitch":48,"bearing":170,"bounds":{"min":[-17,-17,0],"max":[17,17,9]}},"sections":[
        {"id":"near","focus":[-6,4,6],"distance":15,"pitch":28,"bearing":140},
        {"id":"grazing","focus":[0,0,3],"distance":38,"pitch":12,"bearing":180}
    ]})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn million_sample_fixture_is_bounded_and_finite() {
        let fixture = make(DataKey::new(NS, "test").unwrap(), false).unwrap();
        assert_eq!(fixture.pages.len(), 16);
        assert_eq!(
            fixture
                .pages
                .iter()
                .map(|(page, _)| u64::from(page.items))
                .sum::<u64>(),
            u64::from(POINTS)
        );
        for (index, (page, bytes)) in fixture.pages.iter().enumerate() {
            assert!(page.items <= PAGE);
            assert_eq!(page.address.page_id, index as u64);
            assert_eq!(page.byte_len, bytes.len() as u64);
            assert_eq!(page.crc32, page_crc32(bytes));
            let decoded = page.decode_cloud(&schema(), bytes).unwrap();
            assert_eq!(decoded.count, page.items);
            for (offset, bytes) in decoded.columns[1].values.chunks_exact(8).enumerate() {
                assert_eq!(
                    u64::from_le_bytes(bytes.try_into().unwrap()),
                    u64::MAX - (index as u64 * u64::from(PAGE) + offset as u64)
                );
            }
        }
        for index in 0..POINTS {
            let (xyz, rgba) = sample(index, 1000);
            assert!(xyz.into_iter().all(f32::is_finite));
            assert!((0.9..8.0).contains(&xyz[2]));
            assert_eq!(rgba[3], 255);
        }
    }

    #[test]
    fn five_million_source_fixture_preserves_colors_ids_and_bounded_pages() {
        let total = 5_000_000;
        let fixture = make_sized(DataKey::new(NS, "five-million").unwrap(), total).unwrap();
        assert_eq!(fixture.pages.len(), 77);
        assert_eq!(fixture.scans[0].source_count, u64::from(total));
        let mut count = 0;
        for (descriptor, bytes) in &fixture.pages {
            assert!(descriptor.items <= PAGE);
            assert_eq!(descriptor.items, PAGE.min(total - count));
            assert_eq!(descriptor.address.page_id, u64::from(count / PAGE));
            assert_eq!(descriptor.crc32, page_crc32(bytes));
            let decoded = descriptor.decode_cloud(&schema(), bytes).unwrap();
            for index in [0, descriptor.items - 1] {
                let id = &decoded.columns[1].values[index as usize * 8..index as usize * 8 + 8];
                assert_eq!(
                    u64::from_le_bytes(id.try_into().unwrap()),
                    u64::MAX - u64::from(count + index)
                );
                let (xyz, rgba) = sample(count + index, 2237);
                assert!(xyz.into_iter().all(f32::is_finite));
                assert!(xyz[0].abs() <= 15.0 && xyz[1].abs() <= 15.0);
                assert_eq!(
                    &decoded.columns[2].values[index as usize * 4..index as usize * 4 + 4],
                    &rgba
                );
            }
            count += descriptor.items;
        }
        assert_eq!(count, total);
        assert!(make_sized(DataKey::new(NS, "too-large").unwrap(), total + 1).is_err());
    }

    #[test]
    fn empty_observer_has_no_pages_or_scans() {
        let fixture = make(DataKey::new(OBSERVER, "empty").unwrap(), true).unwrap();
        assert!(fixture.pages.is_empty());
        assert!(fixture.scans.is_empty());
    }

    #[test]
    fn scene_retains_caller_color_and_opaque_depth() {
        let p = presentation();
        assert!(matches!(p.color, ColorMode::Attribute { .. }));
        assert_eq!(p.opacity, 1.0);
        assert_eq!(p.blur, 0.0);
        assert!(p.glow > 0.0 && p.glow < 0.4);
    }
}
