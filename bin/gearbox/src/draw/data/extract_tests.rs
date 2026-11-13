use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{
    ecs::system::System,
    render::{MainWorld, sync_component::SyncComponent},
};
use gearbox_api::draw_data::CpuBudget;

use super::*;

#[derive(Component)]
struct Source {
    value: u32,
    owner: Arc<AtomicUsize>,
    budget: CpuBudget,
}

#[derive(Component)]
struct Output {
    value: u32,
    transform: Mat4,
    visible: bool,
    scratch: u64,
    _owner: Arc<AtomicUsize>,
}

impl SyncComponent for Source {
    type Target = Output;
}

impl ExtractComponent for Source {
    type QueryData = (
        &'static Self,
        &'static GlobalTransform,
        &'static InheritedVisibility,
    );
    type QueryFilter = ();
    type Out = Output;

    fn extract_component(
        (source, transform, visibility): QueryItem<'_, '_, Self::QueryData>,
    ) -> Option<Output> {
        Some(Output {
            value: source.value,
            transform: transform.to_matrix(),
            visible: visibility.get(),
            scratch: source.budget.usage().scratch_bytes,
            _owner: source.owner.clone(),
        })
    }
}

impl Budgeted for Source {
    fn denied((source, _, _): QueryItem<'_, '_, Self::QueryData>, _: &DrawingBackend) {
        source.owner.fetch_add(1, Ordering::Relaxed);
    }
}

fn setup() -> (World, DrawingBackend, Entity, Entity, Arc<AtomicUsize>) {
    let backend = DrawingBackend::default();
    let mut world = World::new();
    let mut main = MainWorld::default();
    main.insert_resource(Shared::for_tests(backend.clone(), Handle::default()));
    let owner = Arc::new(AtomicUsize::new(0));
    let source = main
        .spawn((
            Source {
                value: 7,
                owner: owner.clone(),
                budget: backend.budget().clone(),
            },
            GlobalTransform::IDENTITY,
            InheritedVisibility::VISIBLE,
        ))
        .id();
    let mut target = world.spawn_empty().id();
    if target == source {
        target = world.spawn_empty().id();
    }
    main.entity_mut(source).insert(RenderEntity::from(target));
    assert_ne!(source, target);
    world.insert_resource(main);
    (world, backend, source, target, owner)
}

#[test]
fn extraction_is_charged_before_cloning_and_until_deferred_insertion() {
    let (mut world, backend, _, target, owner) = setup();
    let mut system = IntoSystem::into_system(extract::<Source>);
    system.initialize(&mut world);
    let before = backend.budget().usage();
    let bytes = (size_of::<List<(Entity, Output)>>() + size_of::<(Entity, Output)>()) as u64;
    let owners = Arc::strong_count(&owner);
    system
        .run_without_applying_deferred((), &mut world)
        .unwrap();
    assert!(world.get::<Output>(target).is_none());
    assert_eq!(backend.budget().usage().allocated, before.allocated + bytes);
    assert_eq!(Arc::strong_count(&owner), owners + 1);
    system.apply_deferred(&mut world);
    let output = world.get::<Output>(target).unwrap();
    assert_eq!(output.value, 7);
    assert_eq!(output.scratch, before.scratch_bytes + bytes);
    assert_eq!(backend.budget().usage().allocated, before.allocated);
}

#[test]
fn denied_extraction_preserves_previous_values_and_recovers_transform_and_visibility() {
    let (mut world, backend, source, target, owner) = setup();
    let mut system = IntoSystem::into_system(extract::<Source>);
    system.initialize(&mut world);
    system.run((), &mut world).unwrap();
    let transform = GlobalTransform::from_translation(Vec3::new(4.0, 5.0, 6.0));
    {
        let mut main = world.resource_mut::<MainWorld>();
        main.get_mut::<Source>(source).unwrap().value = 9;
        main.entity_mut(source)
            .insert((transform, InheritedVisibility::HIDDEN));
    }
    let before = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(before.limit - before.committed())
        .unwrap();
    let owners = Arc::strong_count(&owner);
    system.run((), &mut world).unwrap();
    let output = world.get::<Output>(target).unwrap();
    assert_eq!(output.value, 7);
    assert_eq!(output.transform, Mat4::IDENTITY);
    assert!(output.visible);
    assert_eq!(owner.load(Ordering::Relaxed), 1);
    assert_eq!(Arc::strong_count(&owner), owners);
    assert_eq!(backend.budget().usage().allocated, before.allocated);
    drop(pressure);
    system.run((), &mut world).unwrap();
    let output = world.get::<Output>(target).unwrap();
    assert_eq!(output.value, 9);
    assert_eq!(output.transform, transform.to_matrix());
    assert!(!output.visible);
    assert_eq!(backend.budget().usage().allocated, before.allocated);
}

#[test]
fn empty_extraction_never_reserves_the_previous_frame_capacity() {
    let (mut world, backend, source, _, owner) = setup();
    let mut system = IntoSystem::into_system(extract::<Source>);
    system.initialize(&mut world);
    system.run((), &mut world).unwrap();
    world.resource_mut::<MainWorld>().despawn(source);
    let before = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(before.limit - before.committed())
        .unwrap();
    for _ in 0..3 {
        system.run((), &mut world).unwrap();
        assert_eq!(backend.budget().usage().allocated, before.allocated);
        assert_eq!(owner.load(Ordering::Relaxed), 0);
    }
    drop(pressure);
}

#[test]
fn discarded_deferred_extraction_releases_scratch_and_component_owners() {
    let (mut world, backend, source, target, owner) = setup();
    let before = backend.budget().usage().allocated;
    let mut system = IntoSystem::into_system(extract::<Source>);
    system.initialize(&mut world);
    system
        .run_without_applying_deferred((), &mut world)
        .unwrap();
    world.resource_mut::<MainWorld>().despawn(source);
    assert_eq!(Arc::strong_count(&owner), 2);
    drop(system);
    assert_eq!(backend.budget().usage().allocated, before);
    assert_eq!(Arc::strong_count(&owner), 1);
    assert!(world.get::<Output>(target).is_none());
}

#[test]
fn actual_bevy_extractor_bypasses_pressure_but_drawing_extractor_denies_it() {
    for budgeted in [false, true] {
        let (world, backend, _, target, owner) = setup();
        let mut app = App::new();
        let mut render = bevy::app::SubApp::new();
        *render.world_mut() = world;
        render.init_resource::<bevy::ecs::schedule::Schedules>();
        app.insert_sub_app(RenderApp, render);
        app.add_plugins(bevy::render::sync_world::SyncWorldPlugin);
        if budgeted {
            install::<Source>(&mut app);
        } else {
            app.add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<
                Source,
            >::default());
        }
        let before = backend.budget().usage();
        let pressure = backend
            .budget()
            .reserve(before.limit - before.committed())
            .unwrap();
        let render = app.sub_app_mut(RenderApp);
        render.world_mut().run_schedule(ExtractSchedule);
        assert_eq!(render.world().get::<Output>(target).is_some(), !budgeted);
        assert_eq!(owner.load(Ordering::Relaxed), usize::from(budgeted));
        assert_eq!(backend.budget().usage().allocated, before.allocated);
        drop(pressure);
    }
}
