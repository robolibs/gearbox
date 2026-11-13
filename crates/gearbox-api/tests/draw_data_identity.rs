use std::sync::Arc;

use gearbox_api::draw_data::{DrawingStore, IdentityError};
use gearbox_api::wire::draw_data::{DataKey, DatasetKind, DatasetRevision};
use gearbox_api::{DrawEntity, draw_kind, draw_op};

fn key(ns: &str, id: &str) -> DataKey {
    DataKey::new(ns, id).unwrap()
}
fn tuple(key: &DataKey) -> (String, String) {
    (key.namespace.clone(), key.id.clone())
}
fn store() -> DrawingStore<Vec<u64>> {
    DrawingStore::new(4, 8).unwrap()
}

#[test]
fn retirement_invalidates_every_generation_but_not_neighbors_or_recreated_keys() {
    let mut s = store();
    let key = key("maps/room", "cloud");
    let neighbor = DataKey::new("maps/roommate", "cloud").unwrap();
    let first = publish(&mut s, &key, DatasetKind::Cloud, None, &[1]);
    let old = s.get(&tuple(&key)).unwrap().lifetime.clone();
    publish(&mut s, &neighbor, DatasetKind::Cloud, None, &[3]);
    let other = s.get(&tuple(&neighbor)).unwrap().lifetime.clone();
    publish(&mut s, &key, DatasetKind::Cloud, Some(first), &[2]);
    let next = s.get(&tuple(&key)).unwrap().lifetime.clone();
    assert!(old.is_live() && next.is_live());
    assert_eq!(s.clear("/maps/room/"), Ok(1));
    assert!(!old.is_live() && !next.is_live());
    assert!(other.is_live());
    let recreated = publish(&mut s, &key, DatasetKind::Cloud, None, &[4]);
    assert_ne!(first.incarnation, recreated.incarnation);
    let fresh = s.get(&tuple(&key)).unwrap().lifetime.clone();
    assert!(fresh.is_live());
    assert!(!old.is_live() && !next.is_live());
    assert_eq!(s.delete(&tuple(&key)), Ok(1));
    assert!(!fresh.is_live());
    assert!(other.is_live());
}
fn publish(
    store: &mut DrawingStore<Vec<u64>>,
    key: &DataKey,
    kind: DatasetKind,
    base: Option<DatasetRevision>,
    values: &[u64],
) -> DatasetRevision {
    let permit = store.begin_dataset(key, kind, base).unwrap();
    store
        .commit_dataset(permit, Arc::new(values.to_vec()))
        .unwrap()
}
fn chunks(ns: &str, id: &str) -> [DrawEntity; 2] {
    let mut first = DrawEntity::points(ns, id, &[[1.0; 3]]);
    first.op = draw_op::APPEND;
    first.seq = 7;
    first.last = 0;
    let mut last = first.clone();
    last.chunk_offset = 1;
    last.last = 1;
    [first, last]
}

#[test]
fn legacy_upsert_completed_append_and_mismatched_extend_keep_replacement_semantics() {
    let mut s = store();
    s.apply_legacy(DrawEntity::points("a", "b", &[[0.0; 3]]), 0.0)
        .unwrap();
    let mut lines = DrawEntity::points("a", "b", &[[1.0; 3], [2.0; 3]]);
    lines.kind = draw_kind::LINES;
    s.apply_legacy(lines.clone(), 0.0).unwrap();
    assert_eq!(
        s.legacy().get("a", "b").unwrap().entity.kind,
        draw_kind::LINES
    );
    let [first, last] = chunks("a", "b");
    s.apply_legacy(first, 0.0).unwrap();
    assert_eq!(
        s.legacy().get("a", "b").unwrap().entity.kind,
        draw_kind::LINES
    );
    s.apply_legacy(last, 0.0).unwrap();
    assert_eq!(
        s.legacy().get("a", "b").unwrap().entity.kind,
        draw_kind::POINTS
    );
    lines.op = draw_op::EXTEND;
    s.apply_legacy(lines, 0.0).unwrap();
    assert_eq!(
        s.legacy().get("a", "b").unwrap().entity.kind,
        draw_kind::LINES
    );
}

#[test]
fn data_cannot_take_a_live_or_incomplete_legacy_key() {
    let mut s = store();
    let k = key("a", "b");
    let [first, last] = chunks("a", "b");
    s.apply_legacy(first, 0.0).unwrap();
    assert_eq!(
        s.begin_dataset(&k, DatasetKind::Cloud, None),
        Err(IdentityError::KindConflict)
    );
    s.apply_legacy(last, 0.0).unwrap();
    assert_eq!(
        s.begin_dataset(&k, DatasetKind::Cloud, None),
        Err(IdentityError::KindConflict)
    );
    assert_eq!((s.dataset_len(), s.pending_len()), (0, 0));
    assert_eq!(s.delete(&tuple(&k)).unwrap(), 1);
    assert!(s.begin_dataset(&k, DatasetKind::Cloud, None).is_ok());
}

#[test]
fn every_legacy_mutation_rejects_live_and_reserved_dataset_ownership() {
    for committed in [false, true] {
        let mut s = store();
        let k = key("a", "b");
        let permit = s.begin_dataset(&k, DatasetKind::Cloud, None).unwrap();
        if committed {
            s.commit_dataset(permit, Arc::new(vec![42])).unwrap();
        }
        for op in [
            draw_op::UPSERT,
            draw_op::APPEND,
            draw_op::EXTEND,
            draw_op::UPDATE,
            draw_op::PATCH,
        ] {
            let mut legacy = DrawEntity::points("/a/", "b", &[[0.0; 3]]);
            legacy.op = op;
            assert_eq!(
                s.apply_legacy(legacy, 0.0),
                Err(IdentityError::KindConflict)
            );
            assert_eq!(s.legacy().pending_len(), 0);
            assert!(s.legacy().is_empty());
        }
        if committed {
            assert_eq!(s.get(&tuple(&k)).unwrap().payload.as_ref(), &[42]);
        }
    }
}

#[test]
fn dataset_kind_changes_require_delete_and_recreate_gets_a_new_incarnation() {
    let mut s = store();
    let k = key("a", "b");
    let first = publish(&mut s, &k, DatasetKind::Cloud, None, &[0]);
    assert_eq!(
        s.begin_dataset(&k, DatasetKind::Octree, Some(first)),
        Err(IdentityError::KindConflict)
    );
    assert_eq!(s.delete(&tuple(&k)).unwrap(), 1);
    let second = publish(&mut s, &k, DatasetKind::Octree, None, &[1]);
    assert_ne!(first.incarnation, second.incarnation);
    assert_eq!(second.generation, 1);
}

#[test]
fn only_one_of_two_concurrent_exact_base_commits_can_win() {
    let mut s = store();
    let k = key("a", "b");
    let first = publish(&mut s, &k, DatasetKind::Cloud, None, &[0]);
    let a = s
        .begin_dataset(&k, DatasetKind::Cloud, Some(first))
        .unwrap();
    let b = s
        .begin_dataset(&k, DatasetKind::Cloud, Some(first))
        .unwrap();
    let second = s.commit_dataset(b, Arc::new(vec![2])).unwrap();
    assert_eq!(
        second,
        DatasetRevision {
            incarnation: first.incarnation,
            generation: 2
        }
    );
    assert_eq!(
        s.commit_dataset(a, Arc::new(vec![1])),
        Err(IdentityError::GenerationConflict)
    );
    assert_eq!(s.get(&tuple(&k)).unwrap().payload.as_ref(), &[2]);
    assert_eq!(s.pending_len(), 0);
    assert_eq!(
        s.begin_dataset(&k, DatasetKind::Cloud, Some(first)),
        Err(IdentityError::GenerationConflict)
    );
}

#[test]
fn competing_initial_commits_and_cancelled_permits_do_not_reuse_identity() {
    let mut s = store();
    let k = key("a", "b");
    let a = s.begin_dataset(&k, DatasetKind::Cloud, None).unwrap();
    let b = s.begin_dataset(&k, DatasetKind::Cloud, None).unwrap();
    s.commit_dataset(a, Arc::new(vec![1])).unwrap();
    assert_eq!(
        s.commit_dataset(b, Arc::new(vec![2])),
        Err(IdentityError::GenerationConflict)
    );
    assert_eq!(
        s.commit_dataset(a, Arc::new(vec![3])),
        Err(IdentityError::Cancelled)
    );
    assert!(!s.cancel(b));
    s.clear("").unwrap();
    let next = s.begin_dataset(&k, DatasetKind::Cloud, None).unwrap();
    assert_ne!(a, next);
    assert!(s.cancel(next));
    assert_eq!(
        s.commit_dataset(next, Arc::new(vec![])),
        Err(IdentityError::Cancelled)
    );
}

#[test]
fn legacy_delete_cancels_all_dataset_permits_before_recreation() {
    let mut s = store();
    let k = key("a", "b");
    let first = publish(&mut s, &k, DatasetKind::Cloud, None, &[0]);
    let late = s
        .begin_dataset(&k, DatasetKind::Cloud, Some(first))
        .unwrap();
    assert_eq!(
        s.apply_legacy(DrawEntity::delete("a", "b"), 0.0).unwrap(),
        1
    );
    let second = publish(&mut s, &k, DatasetKind::Cloud, None, &[3]);
    assert_ne!(first.incarnation, second.incarnation);
    assert_eq!(
        s.commit_dataset(late, Arc::new(vec![2])),
        Err(IdentityError::Cancelled)
    );
    assert_eq!(s.get(&tuple(&k)).unwrap().payload.as_ref(), &[3]);
}

#[test]
fn namespace_clear_removes_both_owners_and_permits_but_preserves_neighbors() {
    let mut s = store();
    s.apply_legacy(DrawEntity::points("a", "legacy", &[[0.0; 3]]), 0.0)
        .unwrap();
    let [first, last] = chunks("a/sub", "pending-legacy");
    s.apply_legacy(first, 0.0).unwrap();
    publish(
        &mut s,
        &key("a/sub", "data"),
        DatasetKind::Cloud,
        None,
        &[1],
    );
    publish(&mut s, &key("ab", "data"), DatasetKind::Cloud, None, &[2]);
    let late = s
        .begin_dataset(&key("a/sub", "pending-data"), DatasetKind::Cloud, None)
        .unwrap();
    let neighbor = s
        .begin_dataset(&key("ab", "pending-data"), DatasetKind::Cloud, None)
        .unwrap();
    assert_eq!(s.apply_legacy(DrawEntity::clear("/a/"), 0.0).unwrap(), 2);
    assert!(s.legacy().is_empty());
    assert!(s.apply_legacy(last, 0.0).is_err());
    assert_eq!(
        s.commit_dataset(late, Arc::new(vec![])),
        Err(IdentityError::Cancelled)
    );
    assert_eq!(
        s.get(&("ab".into(), "data".into()))
            .unwrap()
            .payload
            .as_ref(),
        &[2]
    );
    assert!(s.commit_dataset(neighbor, Arc::new(vec![3])).is_ok());
    s.clear("").unwrap();
    assert_eq!(
        (
            s.dataset_len(),
            s.pending_len(),
            s.legacy().len(),
            s.legacy().pending_len()
        ),
        (0, 0, 0, 0)
    );
}

#[test]
fn identity_limits_include_reserved_keys_without_charging_existing_keys_twice() {
    let mut s: DrawingStore<Vec<u64>> = DrawingStore::new(1, 2).unwrap();
    let k = key("a", "b");
    let a = s.begin_dataset(&k, DatasetKind::Cloud, None).unwrap();
    assert_eq!(
        s.begin_dataset(&key("a", "c"), DatasetKind::Cloud, None),
        Err(IdentityError::Budget)
    );
    let b = s.begin_dataset(&k, DatasetKind::Cloud, None).unwrap();
    assert_eq!(
        s.begin_dataset(&k, DatasetKind::Cloud, None),
        Err(IdentityError::Budget)
    );
    s.cancel(b);
    let revision = s.commit_dataset(a, Arc::new(vec![])).unwrap();
    assert!(
        s.begin_dataset(&k, DatasetKind::Cloud, Some(revision))
            .is_ok()
    );
    s.clear("").unwrap();
    assert!(
        s.begin_dataset(&key("a", "c"), DatasetKind::Octree, None)
            .is_ok()
    );
}

#[test]
fn immutable_snapshots_remain_alive_and_unchanged_after_replacement_and_clear() {
    let mut s = store();
    let k = key("a", "b");
    let first = publish(&mut s, &k, DatasetKind::Cloud, None, &[u64::MAX]);
    let pinned = s.get(&tuple(&k)).unwrap().payload.clone();
    assert_eq!(Arc::strong_count(&pinned), 2);
    publish(&mut s, &k, DatasetKind::Cloud, Some(first), &[1]);
    assert_eq!(Arc::strong_count(&pinned), 1);
    s.clear("").unwrap();
    assert_eq!(pinned.as_ref(), &[u64::MAX]);
}

#[test]
fn legacy_keys_are_not_restricted_to_the_new_dataset_key_limits() {
    let mut s = store();
    let ns = "n".repeat(300);
    let id = "i".repeat(200);
    assert!(DataKey::new(&ns, &id).is_err());
    s.apply_legacy(DrawEntity::points(&ns, &id, &[[0.0; 3]]), 0.0)
        .unwrap();
    assert!(s.legacy().get(&ns, &id).is_some());
    s.apply_legacy(DrawEntity::delete(&ns, &id), 0.0).unwrap();
    assert!(s.legacy().is_empty());
}

#[test]
fn legacy_expiry_keeps_its_existing_simulation_clock_behavior() {
    let mut s = store();
    let k = key("a", "b");
    let [first, _] = chunks("a", "b");
    s.apply_legacy(first, 10.0).unwrap();
    for _ in 0..100 {
        s.expire_legacy(10.0).unwrap();
    }
    assert_eq!(
        s.begin_dataset(&k, DatasetKind::Cloud, None),
        Err(IdentityError::KindConflict)
    );
    s.expire_legacy(40.0).unwrap();
    assert!(s.begin_dataset(&k, DatasetKind::Cloud, None).is_ok());
}

#[test]
fn permits_cannot_cross_store_instances_or_a_reconstructed_registry() {
    let mut a = store();
    let mut b = store();
    let k = key("a", "b");
    let pa = a.begin_dataset(&k, DatasetKind::Cloud, None).unwrap();
    let pb = b.begin_dataset(&k, DatasetKind::Cloud, None).unwrap();
    assert_ne!(pa, pb);
    assert_eq!(
        b.commit_dataset(pa, Arc::new(vec![9])),
        Err(IdentityError::Cancelled)
    );
    assert_eq!(
        a.commit_dataset(pb, Arc::new(vec![9])),
        Err(IdentityError::Cancelled)
    );
    assert_eq!((a.pending_len(), b.pending_len()), (1, 1));
    a.commit_dataset(pa, Arc::new(vec![1])).unwrap();
    b.commit_dataset(pb, Arc::new(vec![2])).unwrap();
}
