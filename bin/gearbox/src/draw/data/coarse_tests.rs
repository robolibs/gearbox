use super::*;
use crate::draw::data::appearance::{Layout, Name, Sample, Selector};
use crate::draw::data::tests::{fixture, stored};

fn geometry(budget: &CpuBudget, count: usize) -> Arc<Geometry> {
    let points: Vec<_> = (0..count)
        .map(|i| {
            if i % 7 == 0 {
                [0.0; 3]
            } else {
                [1e12 + (i % 257) as f64 * 0.125, (i / 257) as f64, 0.5]
            }
        })
        .collect();
    let mut mask = vec![0u8; count.div_ceil(8)];
    for i in 0..count {
        if i % 7 != 0 {
            mask[i / 8] |= 1 << (i % 8);
        }
    }
    let (schema, mut payload) = fixture(&points, mask);
    payload.columns[1].values = (0..count)
        .flat_map(|i| (u64::MAX - i as u64).to_le_bytes())
        .collect();
    Geometry::prepare(budget, &schema, stored(budget, &schema, &payload, [0.0; 3])).unwrap()
}

#[test]
fn representatives_preserve_exact_identity_and_survive_full_source_release() {
    let budget = CpuBudget::new(32 << 20);
    let geometry = geometry(&budget, 65_536);
    let source = Arc::downgrade(geometry.source().unwrap());
    let metadata = geometry.metadata.clone();
    let coarse = &metadata.coarse;
    assert!(
        coarse
            .ordinals
            .values
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    );
    assert_eq!(coarse.positions.values.len(), 256);
    assert!(coarse.positions.values.len() * 100 < geometry.positions.values.len());
    for node in &metadata.hierarchy.nodes.values {
        let compact = coarse.index(node.representative).unwrap() as usize;
        assert_eq!(coarse.ordinals.values[compact], node.representative);
        assert_eq!(
            coarse.source_ids.values[compact],
            geometry.source_id(node.representative as usize).unwrap()
        );
        assert_eq!(
            bytemuck::bytes_of(&coarse.positions.values[compact]),
            bytemuck::bytes_of(&geometry.positions.values[node.representative as usize])
        );
        assert_ne!(geometry.source_row(node.representative as usize).unwrap() % 7, 0);
    }
    assert_eq!(coarse.index(u32::MAX), None);
    let before = budget.usage().committed();
    drop(geometry);
    assert!(source.upgrade().is_none());
    assert!(budget.usage().committed() < before);
    assert!(
        metadata
            .coarse
            .source_ids
            .values
            .iter()
            .all(|id| *id > 1 << 63)
    );
    drop(metadata);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn compact_attributes_preserve_every_field_without_averaging_or_source_ownership() {
    let budget = CpuBudget::new(16 << 20);
    let geometry = geometry(&budget, 4097);
    let count = geometry.positions.values.len();
    let selectors = [
        Selector::Attribute(Name::new("rgba").unwrap()),
        Selector::Scalar(Name::new("scalar").unwrap(), [0.0, 1.0]),
        Selector::Category(Name::new("class").unwrap()),
        Selector::Age,
    ];
    for selector in selectors {
        let mut full = Attributes::new(&budget, Layout::new(&selector, true, count)).unwrap();
        for i in 0..count {
            full.set(
                i,
                &Sample {
                    rgba: 0x8f_7a_19_f1 ^ i as u32,
                    value: (i % 256) as f32 / 256.0,
                    category: u32::MAX - i as u32,
                    radius_m: 0.125 + i as f32,
                    flags: i as u32 & 15,
                    acquisition_ns: [u32::MAX - i as u32, 0xfefefefe],
                    palette_indices: (i as u32 & 0xff) * 0x101 | (i as u32 & 7) << 16,
                },
            );
        }
        let compact = Samples::build(&budget, &geometry.coarse, &full).unwrap();
        let stride = full.layout.stride();
        for (index, ordinal) in geometry.coarse.ordinals.values.iter().map(|o| *o as usize).enumerate() {
            assert_eq!(compact.attributes.sample(index), full.sample(ordinal), "{selector:?}");
            assert_eq!(
                compact.attributes.words.values[index * stride..(index + 1) * stride],
                full.words.values[ordinal * stride..(ordinal + 1) * stride]
            );
        }
        assert_eq!(compact.attributes.layout.count, geometry.coarse.ordinals.values.len());
    }
    drop(geometry);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn coarse_construction_cancellation_and_budget_failure_release_partial_allocations() {
    let budget = CpuBudget::new(16 << 20);
    let geometry = geometry(&budget, 1025);
    let empty = Attributes::new(&budget, Layout::default()).unwrap();
    let before = budget.usage().committed();
    let calls = std::cell::Cell::new(0);
    let result = Representatives::build(&budget, &geometry, &|| {
        calls.set(calls.get() + 1);
        if calls.get() == 3 {
            Err(invalid("cancelled coarse preparation"))
        } else {
            Ok(())
        }
    });
    assert!(result.is_err());
    assert_eq!(calls.get(), 3);
    assert_eq!(budget.usage().committed(), before);
    let held = budget.reserve(budget.usage().limit - before).unwrap();
    assert!(Representatives::build(&budget, &geometry, &|| Ok(())).is_err());
    assert!(Samples::build(&budget, &geometry.coarse, &empty).is_err());
    assert_eq!(budget.usage().committed(), budget.usage().limit);
    drop(held);
    assert_eq!(budget.usage().committed(), before);
    drop(geometry);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn coarse_records_cover_the_spacing_between_representatives() {
    let budget = CpuBudget::new(32 << 20);
    let geometry = geometry(&budget, 65_536);
    let [lo, hi] = geometry.metadata.bounds_m.unwrap();
    let spacing = ((hi[0] - lo[0]) * (hi[1] - lo[1]) / 256.0).sqrt();
    let radius = f64::from(coverage_radius_m(&geometry.metadata));
    assert!((radius - 0.75 * spacing).abs() < 1e-4 * spacing, "{radius} against spacing {spacing}");
    assert!(2.0 * radius > spacing, "neighbouring representatives overlap instead of leaving gaps");
}
