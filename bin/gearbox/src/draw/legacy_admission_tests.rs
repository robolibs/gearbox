use std::sync::Arc;

use super::*;
use gearbox_api::draw_data::CpuBudget;

fn compact_app() -> (App, CpuBudget) {
    let mut app = app();
    let backend = app.world().resource::<DrawStoreRes>().backend().clone();
    let budget = backend.budget().clone();
    let lut = app.world().resource::<TonemappingLuts>().agx.clone();
    app.insert_resource(data::runtime::Shared::for_tests(backend, lut));
    (app, budget)
}

fn cloud(id: &str, x: f32) -> DrawEntity {
    DrawEntity::points("admission", id, &[[x, 0.0, 1.0], [x, 1.0, 2.0]])
        .with_style(gearbox_api::draw_style::SPLAT)
}

fn key(id: &str) -> DrawKey {
    ("admission".into(), id.into())
}

#[test]
fn initial_preparation_denial_does_not_build_a_generic_mesh_and_retries() {
    let (mut app, budget) = compact_app();
    let baseline = counts(&mut app);
    let usage = budget.usage();
    let pressure = budget.reserve(usage.limit - usage.committed()).unwrap();
    apply(&app, cloud("first", 0.0));
    settle(&mut app);
    assert_eq!(counts(&mut app), baseline);
    assert!(app.world().resource::<DrawVisuals>().map.is_empty());
    assert_eq!(budget.usage().allocated, usage.allocated);
    drop(pressure);
    settle(&mut app);
    let visuals = app.world().resource::<DrawVisuals>();
    assert!(visuals.map[&key("first")].compact.is_some());
    assert!(visuals.map[&key("first")].compact_node.is_some());
    assert_eq!(counts(&mut app)[1], baseline[1]);
}

#[test]
fn failed_replacement_preserves_revision_pose_records_and_neighbor() {
    let (mut app, budget) = compact_app();
    apply(&app, cloud("first", 0.0));
    apply(&app, cloud("neighbor", 5.0));
    settle(&mut app);
    let old = app.world().resource::<DrawVisuals>();
    let first = &old.map[&key("first")];
    let (revision, node, pose) = (first.revision, first.compact_node, first.placement.pose);
    let records = first.compact.clone().unwrap();
    let neighbor = old.map[&key("neighbor")].compact.clone().unwrap();
    let baseline = counts(&mut app);
    let usage = budget.usage();
    let pressure = budget.reserve(usage.limit - usage.committed()).unwrap();
    apply(
        &app,
        cloud("first", 2.0).with_pose([5.0, 6.0, 7.0], [0.0, 0.0, 0.0, 1.0]),
    );
    settle(&mut app);
    let visuals = app.world().resource::<DrawVisuals>();
    let current = &visuals.map[&key("first")];
    assert_eq!(
        (
            current.revision,
            current.compact_node,
            current.placement.pose
        ),
        (revision, node, pose)
    );
    assert!(Arc::ptr_eq(current.compact.as_ref().unwrap(), &records));
    assert!(Arc::ptr_eq(
        visuals.map[&key("neighbor")].compact.as_ref().unwrap(),
        &neighbor
    ));
    assert_eq!(counts(&mut app), baseline);
    assert_eq!(budget.usage().allocated, usage.allocated);
    drop(pressure);
    settle(&mut app);
    let visuals = app.world().resource::<DrawVisuals>();
    let current = &visuals.map[&key("first")];
    assert!(current.revision > revision);
    assert_eq!(current.compact_node, node);
    assert_ne!(current.placement.pose, pose);
    assert!(!Arc::ptr_eq(current.compact.as_ref().unwrap(), &records));
    assert!(Arc::ptr_eq(
        visuals.map[&key("neighbor")].compact.as_ref().unwrap(),
        &neighbor
    ));
    assert_eq!(counts(&mut app), baseline);
}

#[test]
fn node_denial_after_successful_source_preparation_preserves_both_render_paths() {
    for compact in [false, true] {
        let (mut app, budget) = compact_app();
        let original = if compact {
            cloud("first", 0.0)
        } else {
            DrawEntity::lines("admission", "first", &[[0.0; 3], [1.0; 3]])
        };
        apply(&app, original);
        settle(&mut app);
        let old = &app.world().resource::<DrawVisuals>().map[&key("first")];
        let (revision, node, content) = (old.revision, old.compact_node, old.content);
        let records = old.compact.clone();
        let baseline = counts(&mut app);
        let usage = budget.usage();
        let replacement = cloud("first", 3.0);
        let prepared = data::legacy::Points::prepare(
            &budget,
            &replacement,
            "admission",
            "first",
            records.as_deref(),
        )
        .unwrap()
        .unwrap();
        let preparation = budget.usage().allocated - usage.allocated;
        assert!(preparation > 0);
        drop(prepared);
        assert_eq!(budget.usage().allocated, usage.allocated);
        let pressure = budget
            .reserve(usage.limit - usage.committed() - preparation)
            .unwrap();
        apply(&app, replacement);
        settle(&mut app);
        let current = &app.world().resource::<DrawVisuals>().map[&key("first")];
        assert_eq!(
            (current.revision, current.compact_node, current.content),
            (revision, node, content)
        );
        assert_eq!(current.compact.is_some(), compact);
        if let Some(records) = &records {
            assert!(Arc::ptr_eq(current.compact.as_ref().unwrap(), records));
        }
        assert_eq!(counts(&mut app), baseline);
        assert_eq!(budget.usage().allocated, usage.allocated);
        drop(pressure);
        settle(&mut app);
        let current = &app.world().resource::<DrawVisuals>().map[&key("first")];
        assert!(current.revision > revision);
        assert!(current.compact.is_some());
        assert!(current.parts.iter().all(Option::is_none));
        assert_eq!(counts(&mut app)[1], 0);
    }
}

#[test]
fn clear_cancels_failed_draws_and_releases_existing_compact_owners_under_pressure() {
    let (mut app, budget) = compact_app();
    let baseline = budget.usage().allocated;
    let assets = counts(&mut app);
    apply(&app, cloud("first", 0.0));
    settle(&mut app);
    let usage = budget.usage();
    let pressure = budget.reserve(usage.limit - usage.committed()).unwrap();
    apply(&app, cloud("first", 2.0));
    apply(&app, cloud("new", 4.0));
    settle(&mut app);
    apply(&app, DrawEntity::clear("admission"));
    settle(&mut app);
    assert_eq!(counts(&mut app), assets);
    assert_eq!(budget.usage().allocated, baseline);
    assert!(app.world().resource::<DrawVisuals>().map.is_empty());
    drop(pressure);
    settle(&mut app);
    assert_eq!(counts(&mut app), assets);
    assert_eq!(budget.usage().allocated, baseline);
}

#[test]
fn unsupported_style_still_replaces_compact_data_with_the_generic_renderer() {
    let (mut app, budget) = compact_app();
    let baseline = budget.usage().allocated;
    apply(&app, cloud("first", 0.0));
    settle(&mut app);
    apply(
        &app,
        cloud("first", 0.0).with_style(gearbox_api::draw_style::HOLO),
    );
    settle(&mut app);
    let current = &app.world().resource::<DrawVisuals>().map[&key("first")];
    assert!(current.compact.is_none());
    assert!(current.compact_node.is_none());
    assert!(current.parts.iter().any(Option::is_some));
    assert!(counts(&mut app)[1] > 0);
    assert_eq!(budget.usage().allocated, baseline);
}
