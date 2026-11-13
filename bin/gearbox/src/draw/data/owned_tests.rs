use super::*;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

#[test]
fn list_capacity_and_draining_ownership_survive_until_last_owner() {
    let budget = CpuBudget::new(1024);
    let mut list = List::new(&budget, 4, MemoryClass::Scratch).unwrap();
    for value in 0..4u64 {
        list.push(value).unwrap();
    }
    assert!(list.push(5).is_err());
    let charged = budget.usage().committed();
    assert_eq!(charged, (32 + size_of::<List<u64>>()) as u64);
    let mut iter = list.into_iter();
    assert_eq!(iter.next(), Some(0));
    assert_eq!(budget.usage().committed(), charged);
    drop(iter);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn list_rejects_overflow_and_budget_before_allocation() {
    let budget = CpuBudget::new(16);
    assert!(List::<u64>::new(&budget, 8, MemoryClass::Scratch).is_err());
    assert!(List::<u64>::new(&budget, usize::MAX, MemoryClass::Scratch).is_err());
    assert_eq!(budget.usage().committed(), 0);
    let list = List::<u64>::new(&budget, 0, MemoryClass::Scratch).unwrap();
    assert!(list.is_empty());
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
struct Collision(u32);
impl Hash for Collision {
    fn hash<H: Hasher>(&self, state: &mut H) {
        0u8.hash(state);
    }
}

#[test]
fn collision_deletion_growth_and_replacement_match_standard_map() {
    let budget = CpuBudget::new(1 << 20);
    let mut owned = Map::default();
    let mut oracle = HashMap::new();
    for step in 0..1800u32 {
        let key = Collision(step.wrapping_mul(73) % 101);
        match step % 7 {
            0 | 1 => assert_eq!(owned.remove(&key), oracle.remove(&key)),
            _ => assert_eq!(
                owned.insert(&budget, key, step).unwrap(),
                oracle.insert(key, step)
            ),
        }
        assert_eq!(owned.len(), oracle.len());
        for i in 0..101 {
            assert_eq!(owned.get(&Collision(i)), oracle.get(&Collision(i)));
        }
    }
    owned.retain(|key, _| key.0 % 2 == 0);
    oracle.retain(|key, _| key.0 % 2 == 0);
    assert_eq!(owned.len(), oracle.len());
    for (key, value) in owned.iter() {
        assert_eq!(Some(value), oracle.get(key));
    }
    owned.clear();
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn failed_map_growth_preserves_entries_and_cleanup_needs_no_budget() {
    let budget = CpuBudget::new(4096);
    let mut map = Map::default();
    map.insert(&budget, 1u64, 11u64).unwrap();
    map.insert(&budget, 2, 22).unwrap();
    let before = budget.usage().committed();
    let pressure = budget.reserve(4096 - before).unwrap();
    assert!(map.insert(&budget, 3, 33).is_err());
    assert_eq!(map.len(), 2);
    assert_eq!(map.insert(&budget, 1, 12).unwrap(), Some(11));
    assert_eq!(map[&2], 22);
    assert!(map.reserve(&budget, usize::MAX).is_err());
    map.retain(|_, _| false);
    assert_eq!(budget.usage().allocated, 0);
    drop(pressure);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn growing_maps_charge_both_old_and_new_capacity() {
    let budget = CpuBudget::new(4096);
    let mut map = Map::default();
    map.insert(&budget, 1u64, 11u64).unwrap();
    map.insert(&budget, 2, 22).unwrap();
    let old = budget.usage().committed();
    map.insert(&budget, 3, 33).unwrap();
    let new = budget.usage().committed();
    assert_eq!(budget.usage().peak, old + new);
    for key in 1..=3 {
        map.remove(&key).unwrap();
    }
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn marked_sweep_handles_duplicates_and_cleanup_without_scratch_allocation() {
    let budget = CpuBudget::new(4096);
    let mut map = Map::default();
    for i in 0..10 {
        map.insert(&budget, Collision(i), i).unwrap();
    }
    let pressure = budget.reserve(4096 - budget.usage().committed()).unwrap();
    let mut removed = Vec::new();
    map.retain_keys(
        [Collision(1), Collision(3), Collision(1), Collision(99)],
        |key, _| removed.push(key.0),
    );
    assert_eq!(map.len(), 2);
    assert_eq!(removed.len(), 8);
    assert_eq!(map[&Collision(1)], 1);
    map.retain_keys([], |_, _| {});
    assert_eq!(budget.usage().allocated, 0);
    drop(pressure);
}

#[test]
fn map_compaction_restores_neighbor_capacity_with_colliding_keys() {
    let budget = CpuBudget::new(16384);
    let mut map = Map::default();
    for i in 0..4 {
        map.insert(&budget, Collision(i), i).unwrap();
    }
    let baseline = budget.usage().allocated;
    for i in 4..15 {
        map.insert(&budget, Collision(i), i).unwrap();
    }
    map.retain(|key, _| key.0 < 4);
    assert!(budget.usage().allocated > baseline);
    map.compact(&budget).unwrap();
    assert_eq!(budget.usage().allocated, baseline);
    for i in 0..4 {
        assert_eq!(map[&Collision(i)], i);
    }
    for i in 4..15 {
        assert!(!map.contains_key(&Collision(i)));
    }
    assert_eq!(map.insert(&budget, Collision(3), 30).unwrap(), Some(3));
    map.insert(&budget, Collision(4), 40).unwrap();
    assert_eq!(map[&Collision(4)], 40);
    map.clear();
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn failed_map_compaction_preserves_neighbors_and_retries_after_pressure_release() {
    let budget = CpuBudget::new(4096);
    let mut map = Map::default();
    for i in 0..3 {
        map.insert(&budget, i, i + 10).unwrap();
    }
    map.remove(&2);
    let before = budget.usage().allocated;
    let pressure = budget.reserve(4096 - before).unwrap();
    assert!(map.compact(&budget).is_err());
    assert_eq!(budget.usage().allocated, before);
    assert_eq!(map[&0], 10);
    assert_eq!(map[&1], 11);
    drop(pressure);
    map.compact(&budget).unwrap();
    assert!(budget.usage().allocated < before);
    assert_eq!(map[&1], 11);
    let pressure = budget.reserve(4096 - budget.usage().committed()).unwrap();
    map.retain(|_, _| false);
    map.compact(&budget).unwrap();
    assert_eq!(budget.usage().allocated, 0);
    drop(pressure);
}

#[test]
fn charged_uniform_serialization_matches_the_production_layout_without_growing() {
    use super::super::style::{Style, owned_uniform, uniform_bytes};
    use gearbox_api::wire::draw_data::Presentation;
    let budget = CpuBudget::new(16384);
    let style = Style::new(&Presentation::default());
    let bytes = owned_uniform(&budget, &style).unwrap();
    assert_eq!(bytes.as_slice(), uniform_bytes(&style));
    assert_eq!(bytes.len(), bytes.capacity());
    assert_eq!(
        budget.usage().scratch_bytes,
        (bytes.len() + size_of::<List<u8>>()) as u64
    );
    let pressure = budget.reserve(16384 - budget.usage().committed()).unwrap();
    assert!(owned_uniform(&budget, &style).is_err());
    assert_eq!(bytes.as_slice(), uniform_bytes(&style));
    drop((bytes, pressure));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn phase_lists_reuse_capacity_without_headroom_and_failed_growth_keeps_previous_items() {
    use super::super::render::phase_lists;
    use bevy::{
        core_pipeline::core_3d::{Opaque3d, Transparent3d},
        prelude::*,
        render::{
            batching::gpu_preprocessing::GpuPreprocessingMode,
            render_phase::{SortedRenderPhase, ViewBinnedRenderPhases},
            view::RetainedViewEntity,
        },
    };
    let budget = CpuBudget::new(16384);
    let entity = Entity::from_raw_u32(1).unwrap();
    let view = RetainedViewEntity::new(entity.into(), None, 0);
    let mut opaque = ViewBinnedRenderPhases::<Opaque3d>::default();
    opaque.prepare_for_new_frame(view, GpuPreprocessingMode::None);
    let opaque = opaque.get_mut(&view).unwrap();
    let mut transparent = SortedRenderPhase::<Transparent3d>::default();
    let mut a = Map::default();
    let mut b = Map::default();
    let (mut items, unused) = phase_lists(
        &budget,
        [1, 0],
        view,
        &mut a,
        &mut b,
        opaque,
        &mut transparent,
    )
    .unwrap();
    assert_eq!(unused.capacity(), 0);
    items.push((entity, entity.into())).unwrap();
    a.insert(&budget, view, items).unwrap();
    let pressure = budget.reserve(16384 - budget.usage().committed()).unwrap();
    assert!(
        phase_lists(
            &budget,
            [2, 0],
            view,
            &mut a,
            &mut b,
            opaque,
            &mut transparent
        )
        .is_err()
    );
    assert_eq!(a[&view].len(), 1);
    let (mut reused, _) = phase_lists(
        &budget,
        [1, 0],
        view,
        &mut a,
        &mut b,
        opaque,
        &mut transparent,
    )
    .unwrap();
    reused.push((entity, entity.into())).unwrap();
    a.insert(&budget, view, reused).unwrap();
    assert_eq!(a[&view].len(), 1);
    drop(pressure);
    let baseline = budget.usage().allocated;
    let (mut grown, _) = phase_lists(
        &budget,
        [8, 0],
        view,
        &mut a,
        &mut b,
        opaque,
        &mut transparent,
    )
    .unwrap();
    grown.push((entity, entity.into())).unwrap();
    a.insert(&budget, view, grown).unwrap();
    let pressure = budget.reserve(16384 - budget.usage().committed()).unwrap();
    let (mut retained, _) = phase_lists(
        &budget,
        [1, 0],
        view,
        &mut a,
        &mut b,
        opaque,
        &mut transparent,
    )
    .unwrap();
    assert_eq!(retained.capacity(), 8);
    retained.push((entity, entity.into())).unwrap();
    a.insert(&budget, view, retained).unwrap();
    drop(pressure);
    let (mut compacted, _) = phase_lists(
        &budget,
        [1, 0],
        view,
        &mut a,
        &mut b,
        opaque,
        &mut transparent,
    )
    .unwrap();
    assert_eq!(compacted.capacity(), 1);
    compacted.push((entity, entity.into())).unwrap();
    a.insert(&budget, view, compacted).unwrap();
    assert_eq!(budget.usage().allocated, baseline);
    a.clear();
    b.clear();
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn phase_metadata_is_released_on_view_or_page_removal_without_scratch_headroom() {
    use super::super::render::prune_phases;
    use bevy::{
        core_pipeline::core_3d::{Opaque3d, Transparent3d},
        prelude::*,
        render::{
            batching::gpu_preprocessing::GpuPreprocessingMode,
            render_phase::{ViewBinnedRenderPhases, ViewSortedRenderPhases},
            view::RetainedViewEntity,
        },
    };
    for page_live in [false, true] {
        let budget = CpuBudget::new(16384);
        let entity = Entity::from_raw_u32(1).unwrap();
        let view = RetainedViewEntity::new(entity.into(), None, 0);
        let mut opaque = ViewBinnedRenderPhases::<Opaque3d>::default();
        opaque.prepare_for_new_frame(view, GpuPreprocessingMode::None);
        let mut transparent = ViewSortedRenderPhases::<Transparent3d>::default();
        let mut a = Map::default();
        let mut b = Map::default();
        let mut items = List::new(&budget, 1, MemoryClass::Metadata).unwrap();
        items.push((entity, entity.into())).unwrap();
        a.insert(&budget, view, items).unwrap();
        let pressure = budget.reserve(16384 - budget.usage().committed()).unwrap();
        prune_phases(
            &mut a,
            &mut b,
            |_| page_live,
            |_| !page_live,
            &mut opaque,
            &mut transparent,
        );
        assert_eq!(budget.usage().allocated, 0);
        drop(pressure);
    }
}
