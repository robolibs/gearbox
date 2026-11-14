use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::Arc;

use super::*;
use crate::draw::data::tests::{fixture, stored};

fn cloud(budget: &CpuBudget, points: &[[f64; 3]], ids: &[u64], mask: Vec<u8>) -> Arc<Geometry> {
    let (schema, mut page) = fixture(points, mask);
    page.columns[1].values = ids.iter().flat_map(|id| id.to_le_bytes()).collect();
    Geometry::prepare(budget, &schema, stored(budget, &schema, &page, [0.0; 3])).unwrap()
}

fn points(count: usize, origin: f64) -> Vec<[f64; 3]> {
    (0..count)
        .map(|index| {
            [
                origin + (index % 113) as f64 * 0.125,
                origin + (index % 71) as f64 * 0.25,
                origin + (index % 29) as f64 * 0.5,
            ]
        })
        .collect()
}

fn ordered_ids(geometry: &Geometry) -> Vec<u64> {
    geometry
        .hierarchy
        .order
        .values
        .iter()
        .map(|&index| geometry.source_id(index as usize).unwrap())
        .collect()
}

fn verify(geometry: &Geometry) {
    let tree = &geometry.hierarchy;
    assert_eq!(tree.order.values.len(), geometry.positions.values.len());
    assert_eq!(
        tree.order.values.iter().copied().collect::<BTreeSet<_>>(),
        (0..geometry.positions.values.len() as u32).collect()
    );
    if tree.nodes.values.is_empty() {
        assert!(tree.order.values.is_empty());
        return;
    }
    assert_eq!(tree.nodes.values[0].count as usize, tree.order.values.len());
    assert_eq!(Some(tree.nodes.values[0].bounds_m), geometry.bounds_m);
    let mut leaf_samples = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut stack = vec![(0usize, 0usize)];
    while let Some((index, depth)) = stack.pop() {
        assert!(visited.insert(index));
        assert!(depth <= 8);
        let node = tree.nodes.values[index];
        let first = node.first as usize;
        let count = node.count as usize;
        let indices = &tree.order.values[first..first + count];
        assert!(indices.contains(&node.representative));
        let representative = geometry
            .source_position(node.representative as usize)
            .unwrap();
        let mut minimum_id = u64::MAX;
        for &ordinal in indices {
            let point = geometry.source_position(ordinal as usize).unwrap();
            let mut distance = 0.0f64;
            for axis in 0..3 {
                assert!(point[axis] >= node.bounds_m[0][axis]);
                assert!(point[axis] <= node.bounds_m[1][axis]);
                distance = distance.hypot(point[axis] - representative[axis]);
            }
            assert!(distance <= node.error_m, "{distance} > {}", node.error_m);
            minimum_id = minimum_id.min(geometry.source_id(ordinal as usize).unwrap());
        }
        assert_eq!(
            geometry.source_id(node.representative as usize),
            Some(minimum_id)
        );
        if let Some([a, b]) = node.children {
            assert!(count > LEAF_POINTS);
            let left = tree.nodes.values[a as usize];
            let right = tree.nodes.values[b as usize];
            assert_eq!(left.first, node.first);
            assert_eq!(right.first, left.first + left.count);
            assert_eq!(left.count + right.count, node.count);
            stack.extend([(a as usize, depth + 1), (b as usize, depth + 1)]);
        } else {
            assert!(count <= LEAF_POINTS);
            for &ordinal in indices {
                assert!(leaf_samples.insert(ordinal));
            }
        }
    }
    assert_eq!(visited.len(), tree.nodes.values.len());
    assert_eq!(leaf_samples.len(), tree.order.values.len());
}

#[test]
fn maximum_page_has_bounded_tree_conservative_bounds_and_exact_source_samples() {
    let budget = CpuBudget::new(32 << 20);
    let count = MAX_PAGE_POINTS as usize;
    let positions = points(count, 0.0);
    let ids: Vec<_> = (0..count).map(|i| u64::MAX - i as u64).collect();
    let geometry = cloud(&budget, &positions, &ids, vec![]);
    verify(&geometry);
    assert_eq!(geometry.hierarchy.nodes.values.len(), 511);
    assert!(
        geometry
            .hierarchy
            .nodes
            .values
            .iter()
            .any(|n| n.error_m > 0.0)
    );
    drop(geometry);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn spatial_and_full_width_id_order_are_independent_of_source_row_permutation() {
    let budget = CpuBudget::new(16 << 20);
    let mut positions = points(1537, 0.0);
    positions[1] = positions[0];
    let mut ids: Vec<_> = (0..positions.len())
        .map(|i| u64::MAX - 7 * i as u64)
        .collect();
    *ids.last_mut().unwrap() = 0;
    let first = cloud(&budget, &positions, &ids, vec![]);
    positions.reverse();
    ids.reverse();
    let reversed = cloud(&budget, &positions, &ids, vec![]);
    assert_eq!(ordered_ids(&first), ordered_ids(&reversed));
    for (a, b) in first
        .hierarchy
        .nodes
        .values
        .iter()
        .zip(&reversed.hierarchy.nodes.values)
    {
        assert_eq!(
            (a.bounds_m, a.first, a.count, a.children, a.error_m),
            (b.bounds_m, b.first, b.count, b.children, b.error_m)
        );
        assert_eq!(
            first.source_id(a.representative as usize),
            reversed.source_id(b.representative as usize)
        );
    }
    verify(&first);
    verify(&reversed);
}

#[test]
fn spatial_partitioning_is_allocation_free_and_independent_of_input_order() {
    let budget = CpuBudget::new(32 << 20);
    for count in [0, 1, 257, 65_536] {
        let geometry = cloud(
            &budget,
            &points(count, 0.0),
            &(0..count as u64).rev().collect::<Vec<_>>(),
            vec![],
        );
        let expected = geometry.hierarchy.order.values.clone();
        let mut reversed = expected.clone();
        reversed.reverse();
        let (result, allocations) = super::super::test_allocations::count(|| {
            super::super::spatial_order::order(&geometry, &mut reversed, &|| Ok(()))
        });
        result.unwrap();
        assert_eq!(allocations, 0);
        assert_eq!(reversed, expected);
    }
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn duplicate_coordinates_remain_distinct_observations_and_do_not_deepen_the_tree() {
    let budget = CpuBudget::new(16 << 20);
    let count = 4097;
    let geometry = cloud(
        &budget,
        &vec![[1.0, 2.0, 3.0]; count],
        &(0..count as u64).rev().collect::<Vec<_>>(),
        vec![],
    );
    assert_eq!(
        ordered_ids(&geometry),
        (0..count as u64).collect::<Vec<_>>()
    );
    assert!(
        geometry
            .hierarchy
            .nodes
            .values
            .iter()
            .all(|n| n.error_m == 0.0)
    );
    verify(&geometry);
}

#[test]
fn original_f64_bounds_survive_large_offsets_and_subnanometre_clusters() {
    for positions in [
        points(777, 1e15),
        (0..777).map(|i| [i as f64 * 1e-12, -1e-9, 0.0]).collect(),
    ] {
        let budget = CpuBudget::new(8 << 20);
        let ids: Vec<_> = (0..positions.len() as u64).collect();
        verify(&cloud(&budget, &positions, &ids, vec![]));
    }
}

#[test]
fn invalid_and_empty_observations_never_become_representatives() {
    let budget = CpuBudget::new(1 << 20);
    for (positions, ids, mask) in [
        (vec![], vec![], vec![]),
        (vec![[0.0; 3]; 3], vec![0, 1, u64::MAX], vec![0]),
        (
            vec![[1.0; 3], [0.0; 3], [2.0; 3]],
            vec![0, 1, u64::MAX],
            vec![5],
        ),
    ] {
        let geometry = cloud(&budget, &positions, &ids, mask);
        verify(&geometry);
        assert!(!ordered_ids(&geometry).contains(&1));
    }
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn radix_has_fixed_passes_and_matches_lexicographic_order() {
    let mut keys: Vec<_> = (0..4099u64)
        .map(|i| Key {
            morton: i.wrapping_mul(0x9e3779b97f4a7c15) & (u64::MAX >> 1),
            source_id: u64::MAX - i,
            render_index: i as u32,
        })
        .collect();
    keys[1].morton = keys[0].morton;
    let mut expected = keys.clone();
    expected.sort_unstable_by_key(|k| (k.morton, k.source_id));
    let checks = Cell::new(0);
    let mut scratch = vec![Key::default(); keys.len()];
    radix(&mut keys, &mut scratch, &|| {
        checks.set(checks.get() + 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        keys.iter().map(|k| k.render_index).collect::<Vec<_>>(),
        expected.iter().map(|k| k.render_index).collect::<Vec<_>>()
    );
    assert_eq!(
        checks.get(),
        16 * (1 + 2 * keys.len().div_ceil(CHECK_INTERVAL)) + 1
    );
}

#[test]
fn index_storage_and_peak_scratch_are_reserved_and_released_exactly() {
    let source_budget = CpuBudget::new(8 << 20);
    let count = 1025;
    let geometry = cloud(
        &source_budget,
        &points(count, 0.0),
        &(0..count as u64).collect::<Vec<_>>(),
        vec![],
    );
    let key_bytes = (count * size_of::<Key>()) as u64;
    let retained = (2 * count * size_of::<u32>()
        + node_capacity(count) * size_of::<Node>()
        + geometry.hierarchy.nodes.values.len() * size_of::<[f64; 8]>()) as u64;
    for limit in [0, key_bytes - 1, key_bytes, 2 * key_bytes - 1] {
        let budget = CpuBudget::new(limit);
        assert!(matches!(
            Hierarchy::build(&budget, &geometry, &|| Ok(())),
            Err(StorageError::Budget { .. })
        ));
        assert_eq!(budget.usage().committed(), 0);
    }
    let budget = CpuBudget::new(2 * key_bytes);
    let hierarchy = Hierarchy::build(&budget, &geometry, &|| Ok(())).unwrap();
    assert_eq!(budget.usage().allocated, retained);
    assert_eq!(budget.usage().peak, 2 * key_bytes);
    assert_eq!(budget.usage().reserved, 0);
    drop(hierarchy);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn cancellation_at_every_index_checkpoint_discards_partial_owned_work() {
    let source_budget = CpuBudget::new(8 << 20);
    let geometry = cloud(
        &source_budget,
        &points(513, 0.0),
        &(0..513).collect::<Vec<_>>(),
        vec![],
    );
    let budget = CpuBudget::new(8 << 20);
    let calls = Cell::new(0);
    drop(
        Hierarchy::build(&budget, &geometry, &|| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap(),
    );
    for cancel_at in 1..=calls.get() {
        let current = Cell::new(0);
        let result = Hierarchy::build(&budget, &geometry, &|| {
            current.set(current.get() + 1);
            if current.get() == cancel_at {
                Err(StorageError::Conflict)
            } else {
                Ok(())
            }
        });
        assert!(
            matches!(result, Err(StorageError::Conflict)),
            "checkpoint {cancel_at}"
        );
        assert_eq!(current.get(), cancel_at);
        assert_eq!(budget.usage().committed(), 0);
    }
}

#[test]
fn geometry_cancellation_includes_compaction_sorting_and_final_publication() {
    let budget = CpuBudget::new(8 << 20);
    let (schema, page) = fixture(&points(1025, 0.0), vec![]);
    let source = stored(&budget, &schema, &page, [0.0; 3]);
    let baseline = budget.usage().committed();
    let checks = Cell::new(0);
    drop(
        Geometry::prepare_cancellable(&budget, &schema, source.clone(), &|| {
            checks.set(checks.get() + 1);
            Ok(())
        })
        .unwrap(),
    );
    for stop in [1, 3, 5, 9, checks.get() / 2, checks.get() - 1, checks.get()] {
        let current = Cell::new(0);
        let result = Geometry::prepare_cancellable(&budget, &schema, source.clone(), &|| {
            current.set(current.get() + 1);
            if current.get() == stop {
                Err(StorageError::Conflict)
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(StorageError::Conflict)));
        assert_eq!(budget.usage().committed(), baseline);
    }
}

#[test]
fn node_capacity_bounds_every_legal_page_shape() {
    fn needed(count: usize) -> usize {
        if count == 0 {
            0
        } else if count <= LEAF_POINTS {
            1
        } else {
            1 + needed(count / 2) + needed(count - count / 2)
        }
    }
    for count in 0..=MAX_PAGE_POINTS as usize {
        assert!(needed(count) <= node_capacity(count));
        assert!(node_capacity(count) <= 511);
    }
}
