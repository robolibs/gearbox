use super::super::worker::{test_dataset, test_scene, test_snapshot};
use super::*;

fn data(budget: &CpuBudget, generation: u64, empty: bool) -> Arc<Dataset> {
    let mut data = test_dataset(budget, empty);
    Arc::get_mut(&mut data).unwrap().revision.generation = generation;
    data
}

fn select(
    budget: &CpuBudget,
    old: Option<&Arc<DatasetSet>>,
    candidates: &[Arc<Dataset>],
    ready: bool,
) -> Arc<DatasetSet> {
    DatasetSet::select(budget, old, candidates, |_| true, |_| ready).unwrap()
}

fn needed(offered: &Offered, held: &Holds) -> Vec<Arc<Dataset>> {
    let mut values = Vec::new();
    offered.visit_needed(held, |data| values.push(data.clone()));
    values
}

fn view(id: u32, epoch: u64, datasets: Arc<DatasetSet>) -> ViewFrontier {
    ViewFrontier {
        retained: RetainedViewEntity::new(Entity::from_raw_u32(id).unwrap().into(), None, 0),
        identity: ViewIdentity {
            id: id as u64,
            epoch,
        },
        datasets,
        preparing: None,
    }
}

#[test]
fn pending_cut_survives_newer_offers_without_publishing_it_as_active() {
    let budget = CpuBudget::new(16 << 20);
    let old = data(&budget, 1, false);
    let preparing = data(&budget, 2, false);
    let newest = data(&budget, 3, false);
    let mut frontier = view(1, 1, select(&budget, None, &[old], true));
    frontier.preparing = Some(select(&budget, None, &[preparing.clone()], true));
    let mut selection = Selection::new(&budget, 1).unwrap();
    selection.views.push(frontier);
    let selection = Arc::new(selection);
    let offered = Offered {
        snapshot: Some(test_snapshot(&budget, &[newest.clone()])),
        scene: Some(test_scene(&budget, vec![newest])),
        retention: Retention::new(&budget).unwrap(),
    };
    offered.retention.select(Some(selection.clone()));
    let mut generations: Vec<_> = needed(&offered, &offered.retention.read())
        .iter()
        .map(|data| data.revision.generation)
        .collect();
    generations.sort_unstable();
    assert_eq!(generations, [1, 2, 3]);
    assert_eq!(
        selection.views[0].datasets.datasets.read()[0]
            .revision
            .generation,
        1
    );
    let weak = Arc::downgrade(&preparing);
    drop(preparing);
    assert!(weak.upgrade().is_some());
    offered.retention.select(None);
    drop((selection, offered));
    assert!(weak.upgrade().is_none());
    drop(weak);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn pruned_frontier_capacity_is_replaced_without_losing_surviving_data() {
    let budget = CpuBudget::new(16 << 20);
    let first = data(&budget, 1, false);
    let mut values = List::new(&budget, 2, MemoryClass::Metadata).unwrap();
    values.push(first.clone()).unwrap();
    let old = Arc::new(DatasetSet {
        datasets: values.into(),
        _charge: charge(&budget, size_of::<DatasetSet>() + 16).unwrap(),
    });
    let pressure = budget
        .reserve(budget.usage().limit - budget.usage().committed())
        .unwrap();
    assert!(DatasetSet::select(&budget, Some(&old), &[], |_| true, |_| true).is_err());
    assert!(Arc::ptr_eq(&old.datasets.read()[0], &first));
    drop(pressure);
    let compacted = select(&budget, Some(&old), &[], true);
    assert!(!Arc::ptr_eq(&compacted, &old));
    assert_eq!(old.datasets.read().capacity(), 2);
    assert_eq!(compacted.datasets.read().capacity(), 1);
    assert!(Arc::ptr_eq(&compacted.datasets.read()[0], &first));
    assert!(Arc::ptr_eq(
        &compacted,
        &select(&budget, Some(&compacted), &[], true)
    ));
    drop((compacted, old, first));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn cameras_switch_independently_and_skip_superseded_candidates() {
    let budget = CpuBudget::new(16 << 20);
    let first = data(&budget, 1, false);
    let second = data(&budget, 2, false);
    let third = data(&budget, 3, false);
    let initial = select(&budget, None, &[first.clone()], true);
    let a = select(&budget, Some(&initial), &[second.clone()], true);
    let mut b = select(&budget, Some(&initial), &[second], false);
    assert_eq!(a.datasets.read()[0].revision.generation, 2);
    assert!(Arc::ptr_eq(&initial, &b));
    b = select(&budget, Some(&b), &[third.clone()], false);
    assert_eq!(b.datasets.read()[0].revision.generation, 1);
    b = select(&budget, Some(&b), &[third.clone()], true);
    assert!(Arc::ptr_eq(&b.datasets.read()[0], &third));
    assert_eq!(a.datasets.read()[0].revision.generation, 2);
    drop((first, third, initial, a, b));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn missing_or_unready_replacement_preserves_complete_frontier() {
    let budget = CpuBudget::new(16 << 20);
    let first = data(&budget, 1, false);
    let next = data(&budget, 2, false);
    let initial = select(&budget, None, &[first], true);
    assert!(Arc::ptr_eq(
        &initial,
        &select(&budget, Some(&initial), &[], false)
    ));
    assert!(Arc::ptr_eq(
        &initial,
        &select(&budget, Some(&initial), &[next], false)
    ));
    let exhausted = CpuBudget::new(0);
    let candidate = data(&budget, 3, false);
    assert!(
        DatasetSet::select(&exhausted, Some(&initial), &[candidate], |_| true, |_| true).is_err()
    );
    assert_eq!(initial.datasets.read()[0].revision.generation, 1);
}

#[test]
fn clear_and_incarnation_replacement_do_not_keep_old_generations() {
    let budget = CpuBudget::new(16 << 20);
    let first = data(&budget, 1, false);
    let initial = select(&budget, None, &[first], true);
    let cleared = DatasetSet::select(&budget, Some(&initial), &[], |_| false, |_| true).unwrap();
    assert!(cleared.datasets.is_empty());
    let mut replacement = data(&budget, 1, false);
    Arc::get_mut(&mut replacement).unwrap().revision.incarnation = 2;
    let result = DatasetSet::select(
        &budget,
        Some(&initial),
        &[replacement],
        |data| data.revision.incarnation == 2,
        |_| true,
    )
    .unwrap();
    assert_eq!(result.datasets.len(), 1);
    assert_eq!(result.datasets.read()[0].revision.incarnation, 2);
}

#[test]
fn empty_generation_can_replace_nonempty_without_page_nodes() {
    let budget = CpuBudget::new(16 << 20);
    let initial = select(&budget, None, &[data(&budget, 1, false)], true);
    let empty = data(&budget, 2, true);
    let result = DatasetSet::select(
        &budget,
        Some(&initial),
        &[empty.clone()],
        |_| true,
        |data| data.render_pages() == 0,
    )
    .unwrap();
    assert!(Arc::ptr_eq(&result.datasets.read()[0], &empty));
}

#[test]
fn selection_identity_includes_view_incarnation() {
    let budget = CpuBudget::new(16 << 20);
    let datasets = select(&budget, None, &[data(&budget, 1, false)], true);
    let old = view(1, 1, datasets.clone());
    let new = view(1, 2, datasets);
    let mut selection = Selection::new(&budget, 1).unwrap();
    selection.views.push(old.clone());
    assert!(selection.find(old.retained, old.identity).is_some());
    assert!(selection.find(new.retained, new.identity).is_none());
}

#[test]
fn extracted_offer_closes_main_render_selection_race_and_releases_retired_data() {
    let budget = CpuBudget::new(16 << 20);
    let first = data(&budget, 1, false);
    let second = data(&budget, 2, false);
    let third = data(&budget, 3, false);
    let retired = Arc::downgrade(&first);
    let mut offered = Offered {
        snapshot: Some(test_snapshot(&budget, &[second.clone()])),
        scene: Some(test_scene(&budget, vec![first.clone()])),
        retention: Retention::new(&budget).unwrap(),
    };
    let render_input = Offered::extract_resource(&offered);
    offered.scene = Some(test_scene(&budget, vec![second.clone()]));
    let held = offered.retention.read();
    assert!(
        needed(&offered, &held)
            .iter()
            .any(|data| Arc::ptr_eq(data, &first))
    );
    let mut selected = Selection::new(&budget, 1).unwrap();
    selected.views.push(view(
        1,
        1,
        select(&budget, None, &render_input.candidates(), true),
    ));
    offered.retention.select(Some(Arc::new(selected)));
    drop((render_input, held, first));
    Offered::extract_resource(&offered);
    offered.snapshot = Some(test_snapshot(&budget, &[third.clone()]));
    offered.scene = Some(test_scene(&budget, vec![third]));
    let held = offered.retention.read();
    let mut generations: Vec<_> = needed(&offered, &held)
        .iter()
        .map(|data| data.revision.generation)
        .collect();
    generations.sort_unstable();
    assert_eq!(generations, [1, 2, 3]);
    offered.snapshot = Some(test_snapshot(&budget, &[]));
    assert!(needed(&offered, &held).is_empty());
    offered.retention.select(None);
    assert!(retired.upgrade().is_some());
    drop((held, offered, second));
    assert!(retired.upgrade().is_none());
    assert_eq!(budget.usage().committed(), 0);
}
