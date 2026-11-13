use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::Arc;

use super::*;
use crate::draw::data::tests::{fixture, stored};

fn geometry(budget: &CpuBudget, count: usize, reverse: bool) -> Arc<Geometry> {
    let mut points: Vec<_> = (0..count)
        .map(|i| {
            [
                (i % 37) as f64 * 0.125,
                (i % 53) as f64 * 0.25,
                (i % 71) as f64 * 0.5,
            ]
        })
        .collect();
    let mut ids: Vec<_> = (0..count).map(|i| u64::MAX - i as u64 * 7).collect();
    if reverse {
        points.reverse();
        ids.reverse();
    }
    let (schema, mut page) = fixture(&points, vec![]);
    page.columns[1].values = ids.iter().flat_map(|id| id.to_le_bytes()).collect();
    Geometry::prepare(budget, &schema, stored(budget, &schema, &page, [0.0; 3])).unwrap()
}

#[test]
fn odd_and_full_leaves_have_nested_exact_prefixes_with_conservative_errors() {
    let budget = CpuBudget::new(16 << 20);
    for count in [0, 1, 2, 3, 5, 11, 127, 129, 193, 255, 256, 4097] {
        let geometry = geometry(&budget, count, false);
        let tree = &geometry.hierarchy;
        let morton = tree.order.values.clone();
        let rebuilt =
            Refinement::build(&budget, &geometry, &morton, &tree.nodes.values, &|| Ok(())).unwrap();
        assert_eq!(tree.order.values, morton);
        assert_eq!(rebuilt.order.values, tree.refinement.order.values);
        for (index, node) in tree
            .nodes
            .values
            .iter()
            .enumerate()
            .filter(|(_, n)| n.children.is_none())
        {
            let range = node.first as usize..(node.first + node.count) as usize;
            let source = &tree.order.values[range.clone()];
            let ordered = &rebuilt.order.values[range];
            assert_eq!(ordered[0], node.representative);
            assert_eq!(
                ordered.iter().copied().collect::<BTreeSet<_>>(),
                source.iter().copied().collect()
            );
            assert_eq!(ordered.len(), ordered.iter().collect::<BTreeSet<_>>().len());
            let mut previous = f64::INFINITY;
            for level in 0..LEVELS {
                let prefix = &ordered[..(1 << level).min(ordered.len())];
                let error = rebuilt.errors_m.values[index][level];
                assert!(error <= previous);
                previous = error;
                for ordinal in source {
                    let point = geometry.source_position(*ordinal as usize).unwrap();
                    let nearest = prefix
                        .iter()
                        .map(|selected| {
                            let representative =
                                geometry.source_position(*selected as usize).unwrap();
                            (0..3).fold(0.0f64, |distance, axis| {
                                distance.hypot(point[axis] - representative[axis])
                            })
                        })
                        .fold(f64::INFINITY, f64::min);
                    assert!(
                        nearest <= error,
                        "count {count}, node {index}, level {level}: {nearest} > {error}"
                    );
                }
            }
        }
    }
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn progressive_source_ids_and_errors_are_independent_of_row_permutation() {
    let budget = CpuBudget::new(16 << 20);
    let a = geometry(&budget, 4097, false);
    let b = geometry(&budget, 4097, true);
    let ids = |geometry: &Geometry| {
        geometry
            .hierarchy
            .refinement
            .order
            .values
            .iter()
            .map(|ordinal| geometry.source_id(*ordinal as usize).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&a), ids(&b));
    assert_eq!(
        a.hierarchy.refinement.errors_m.values,
        b.hierarchy.refinement.errors_m.values
    );
    drop((a, b));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn metadata_and_scratch_are_exactly_charged_and_cancelled_at_every_checkpoint() {
    let source_budget = CpuBudget::new(16 << 20);
    let geometry = geometry(&source_budget, 513, false);
    let tree = &geometry.hierarchy;
    let retained = (tree.order.values.len() * size_of::<u32>()
        + tree.nodes.values.len() * size_of::<[f64; LEVELS]>()) as u64;
    let peak = retained + size_of::<Scratch>() as u64;
    for limit in [0, retained - 1, peak - 1] {
        let budget = CpuBudget::new(limit);
        assert!(matches!(
            Refinement::build(
                &budget,
                &geometry,
                &tree.order.values,
                &tree.nodes.values,
                &|| Ok(())
            ),
            Err(StorageError::Budget { .. })
        ));
        assert_eq!(budget.usage().committed(), 0);
    }
    let budget = CpuBudget::new(peak);
    let calls = Cell::new(0);
    let result = Refinement::build(
        &budget,
        &geometry,
        &tree.order.values,
        &tree.nodes.values,
        &|| {
            calls.set(calls.get() + 1);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(budget.usage().allocated, retained);
    assert_eq!(budget.usage().peak, peak);
    assert_eq!(budget.usage().reserved, 0);
    drop(result);
    for stop in 1..=calls.get() {
        let current = Cell::new(0);
        let result = Refinement::build(
            &budget,
            &geometry,
            &tree.order.values,
            &tree.nodes.values,
            &|| {
                current.set(current.get() + 1);
                if current.get() == stop {
                    Err(StorageError::Conflict)
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(result, Err(StorageError::Conflict)));
        assert_eq!(budget.usage().committed(), 0);
    }
    drop(geometry);
    assert_eq!(source_budget.usage().committed(), 0);
}
