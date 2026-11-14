use super::*;

#[cfg(feature = "draw-probes")]
#[test]
fn only_explicit_probe_image_camera_joins_the_interactive_view_roster() {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::render::MainWorld;
    let mut main = MainWorld::default();
    let layers = RenderLayers::layer(super::super::super::DRAW_LAYER);
    let primary = main
        .spawn((
            Camera3d::default(),
            ChaseCamera::default(),
            RenderTarget::TextureView(ManualTextureViewHandle(41)),
            layers.clone(),
        ))
        .id();
    let secondary = main
        .spawn((
            Camera3d::default(),
            super::super::probe::ProbeCamera,
            RenderTarget::Image(Handle::<Image>::default().into()),
            layers.clone(),
        ))
        .id();
    main.spawn((
        Camera3d::default(),
        RenderTarget::Image(Handle::<Image>::default().into()),
        layers,
    ));
    let mut render = World::new();
    render.insert_resource(main);
    render.init_resource::<ViewRoster>();
    render.run_system_once(extract_views).unwrap();
    let roster = render.resource::<ViewRoster>();
    assert_eq!(roster.count, 2);
    assert!(!roster.overflow);
    assert_eq!(
        roster.entries[0].unwrap().retained.main_entity.id(),
        primary
    );
    assert_eq!(
        roster.entries[1].unwrap().retained.main_entity.id(),
        secondary
    );
}

fn view(id: u32) -> RegisteredView {
    RegisteredView {
        retained: RetainedViewEntity::new(Entity::from_raw_u32(id).unwrap().into(), None, 0),
        surface: ViewSurface::Window(Entity::from_raw_u32(20).unwrap()),
        identity: ViewIdentity {
            id: id as u64,
            epoch: 1,
        },
    }
}

fn limiter(backend: &DrawingBackend) -> Arc<Limiter> {
    Arc::new(Limiter {
        count: AtomicUsize::new(0),
        _charge: charge(backend.budget(), size_of::<Limiter>() + 16).unwrap(),
    })
}

#[test]
fn absent_datasets_allocate_no_receipt_but_empty_datasets_still_do() {
    use super::super::frontier::{DatasetSet, Retention, Selection, ViewFrontier};
    use super::super::worker::{test_dataset, test_snapshot};
    use bevy::ecs::system::SystemState;
    for (present, count) in [(false, 2), (true, 1), (true, 2)] {
        let backend = DrawingBackend::default();
        let mut world = World::new();
        let mut queries = SystemState::<(
            Query<(Entity, &Node)>,
            Query<(&ExtractedView, &Msaa)>,
            Query<(Entity, &super::super::octree::Node)>,
        )>::new(&mut world);
        let (nodes, views, octree_nodes) = queries.get(&world).unwrap();
        let datasets = if present {
            vec![test_dataset(backend.budget(), true)]
        } else {
            Vec::new()
        };
        let selected =
            DatasetSet::select(backend.budget(), None, &datasets, |_| true, |_| true).unwrap();
        let mut selection = Selection::new(backend.budget(), count).unwrap();
        let registered: Vec<_> = (1..=count).map(|id| view(id as u32)).collect();
        for view in &registered {
            selection.views.push(ViewFrontier {
                retained: view.retained,
                identity: view.identity,
                datasets: selected.clone(),
                preparing: None,
            });
        }
        let frontiers = Frontiers(Some(Arc::new(selection)));
        let offered = Offered {
            snapshot: Some(test_snapshot(backend.budget(), &datasets)),
            scene: None,
            retention: Retention::new(backend.budget()).unwrap(),
        };
        let limiter = limiter(&backend);
        let mut receipts = Receipts {
            views: registered,
            limiter: Some(limiter.clone()),
            ..Default::default()
        };
        let before = backend.budget().usage();
        let pressure = (!present).then(|| {
            backend
                .budget()
                .reserve(before.limit - before.committed())
                .unwrap()
        });
        prepare(
            &mut receipts,
            &backend,
            &frontiers,
            &offered,
            &nodes,
            &views,
            None,
            &octree_nodes,
            None,
        )
        .unwrap();
        assert_eq!(receipts.current.is_some(), present);
        assert_eq!(limiter.count.load(Ordering::Acquire), usize::from(present));
        if let Some(frame) = &receipts.current {
            assert_eq!(frame.entries.len(), count);
            assert!(frame.entries.iter().all(|entry| entry.expected == 0));
            assert!(frame.pages.is_empty());
            let bytes = frame._charge.bytes();
            assert_eq!(backend.budget().usage().allocated - before.allocated, bytes);
            eprintln!("selected empty dataset: {count} views, {bytes} receipt metadata bytes");
            let pending = receipts.current.take().unwrap();
            assert_eq!(limiter.count.load(Ordering::Acquire), 1);
            assert_eq!(backend.budget().usage().allocated - before.allocated, bytes);
            drop(pending);
            assert_eq!(limiter.count.load(Ordering::Acquire), 0);
            assert_eq!(backend.budget().usage().allocated, before.allocated);
        } else {
            assert_eq!(backend.budget().usage().allocated, before.allocated);
        }
        drop(pressure);
    }
}

#[cfg(feature = "draw-probes")]
#[test]
fn probe_pause_drains_real_permits_without_removing_the_selected_empty_dataset() {
    use super::super::frontier::{DatasetSet, Retention, Selection, ViewFrontier};
    use super::super::worker::{test_dataset, test_snapshot};
    use bevy::ecs::system::RunSystemOnce;
    let backend = DrawingBackend::default();
    let datasets = vec![test_dataset(backend.budget(), true)];
    let selected =
        DatasetSet::select(backend.budget(), None, &datasets, |_| true, |_| true).unwrap();
    let view = view(1);
    let mut selection = Selection::new(backend.budget(), 1).unwrap();
    selection.views.push(ViewFrontier {
        retained: view.retained,
        identity: view.identity,
        datasets: selected.clone(),
        preparing: None,
    });
    let probe = Arc::new(super::super::probe::Probe::for_tests());
    let mut shared = Shared::for_tests(backend.clone(), Handle::default());
    shared.probe = Some(probe.clone());
    let mut world = World::new();
    world.insert_resource(shared);
    world.insert_resource(Frontiers(Some(Arc::new(selection))));
    world.insert_resource(Offered {
        snapshot: Some(test_snapshot(backend.budget(), &datasets)),
        scene: None,
        retention: Retention::new(backend.budget()).unwrap(),
    });
    world.insert_resource(Receipts {
        frame: 7,
        views: vec![view],
        limiter: Some(limiter(&backend)),
        ..Default::default()
    });
    let baseline = backend.budget().usage().allocated;
    world.run_system_once(prepare_frame).unwrap();
    let pending = world.resource_mut::<Receipts>().current.take().unwrap();
    assert_eq!(pending.entries.len(), 1);
    probe.configure_receipts(true);
    world.run_system_once(prepare_frame).unwrap();
    assert!(world.resource::<Receipts>().current.is_none());
    world.run_system_once(observe).unwrap();
    assert!(probe.receipt_snapshot().paused);
    assert_eq!(probe.receipt_snapshot().in_flight, 1);
    assert_eq!(
        backend.budget().usage().allocated,
        baseline + pending._charge.bytes()
    );
    drop(pending);
    world.run_system_once(observe).unwrap();
    assert_eq!(probe.receipt_snapshot().in_flight, 0);
    assert_eq!(backend.budget().usage().allocated, baseline);
    assert_eq!(selected.datasets.read().len(), 1);
    probe.configure_receipts(false);
    world.run_system_once(prepare_frame).unwrap();
    world.run_system_once(observe).unwrap();
    assert!(world.resource::<Receipts>().current.is_some());
    assert!(!probe.receipt_snapshot().paused);
    assert_eq!(probe.receipt_snapshot().in_flight, 1);
}

fn frame(backend: &DrawingBackend, empty: bool, views: usize, pages: usize) -> Frame {
    let dataset = super::super::worker::test_dataset(backend.budget(), empty);
    let mut frame = Frame {
        number: 7,
        views: (0..views)
            .map(|i| ViewMark {
                view: view(i as u32 + 1),
                finished: AtomicBool::new(false),
            })
            .collect(),
        entries: (0..views)
            .map(|view| Entry {
                dataset: dataset.clone(),
                view,
                expected: pages,
                selected_items: AtomicU64::new(dataset.pages.iter().map(|page| page.clip.selected).sum()),
                slots: Vec::new(),
                resolved: AtomicBool::new(true),
                drawn: AtomicUsize::new(0),
            })
            .collect(),
        pages: (0..views)
            .flat_map(|v| {
                (0..pages).map(move |i| PageMark {
                    view: v,
                    entity: Entity::from_raw_u32(i as u32 + 50).unwrap(),
                    entry: v,
                    required: super::super::alpha::OPAQUE,
                    drawn: AtomicU8::new(0),
                })
            })
            .collect(),
        octrees: Vec::new(),
        octree_pages: Vec::new(),
        readbacks: Mutex::new(Vec::new()),
        _charge: charge(
            backend.budget(),
            size_of::<Frame>()
                + views * size_of::<ViewMark>()
                + views * size_of::<Entry>()
                + views * pages * size_of::<PageMark>(),
        )
        .unwrap(),
        _permit: Permit::acquire(&limiter(backend)).unwrap(),
    };
    frame
        .pages
        .sort_unstable_by_key(|page| (page.view, page.entity.to_bits()));
    frame
}

#[test]
fn receipt_needs_every_distinct_draw_and_its_own_view_pass() {
    let backend = DrawingBackend::default();
    let frame = frame(&backend, false, 2, 2);
    let first = Entity::from_raw_u32(50).unwrap();
    let second = Entity::from_raw_u32(51).unwrap();
    let a = view(1).retained;
    let b = view(2).retained;
    frame.drawn(a, first, 1);
    frame.drawn(a, first, 1);
    frame.drawn(view(3).retained, second, 1);
    frame.views[0].finished.store(true, Ordering::Release);
    assert!(frame.completion(0).is_none());
    frame.drawn(a, second, 1);
    assert!(frame.completion(0).is_some());
    frame.drawn(b, first, 1);
    frame.drawn(b, second, 1);
    assert!(frame.completion(1).is_none());
    frame.views[1].finished.store(true, Ordering::Release);
    assert_eq!(frame.completion(1).unwrap().target.view, view(2).identity);
}

#[test]
fn empty_frontier_still_needs_a_completed_view_pass() {
    let backend = DrawingBackend::default();
    let baseline = backend.budget().usage().committed();
    let frame = frame(&backend, true, 1, 0);
    assert!(frame.completion(0).is_none());
    frame.views[0].finished.store(true, Ordering::Release);
    let completion = frame.completion(0).unwrap();
    assert_eq!(completion.disposition, FrontierDisposition::Empty);
    assert_eq!(
        (completion.source_samples, completion.selected_items),
        (0, 0)
    );
    drop(frame);
    assert_eq!(backend.budget().usage().committed(), baseline);
}

#[test]
fn mixed_and_single_sample_pages_need_both_passes_without_double_counting() {
    let backend = DrawingBackend::default();
    for order in [[1, 2], [2, 1]] {
        let mut frame = frame(&backend, false, 1, 1);
        frame.pages[0].required = 3;
        frame.views[0].finished.store(true, Ordering::Release);
        let camera = view(1).retained;
        let entity = Entity::from_raw_u32(50).unwrap();
        frame.drawn(camera, entity, 4);
        frame.drawn(camera, entity, order[0]);
        frame.drawn(camera, entity, order[0]);
        assert!(frame.completion(0).is_none());
        frame.drawn(camera, entity, order[1]);
        assert!(frame.completion(0).is_some());
        frame.drawn(camera, entity, order[1]);
        assert_eq!(frame.entries[0].drawn.load(Ordering::Acquire), 1);
    }
}

#[test]
fn wholly_clipped_frontier_requires_draw_and_view_proof_and_is_not_empty() {
    let backend = DrawingBackend::default();
    let mut frame = frame(&backend, false, 1, 1);
    let data = Arc::get_mut(&mut frame.entries[0].dataset).unwrap();
    Arc::get_mut(&mut data.pages[0]).unwrap().clip.selected = 0;
    frame.entries[0].selected_items = AtomicU64::new(0);
    assert!(frame.completion(0).is_none());
    frame.views[0].finished.store(true, Ordering::Release);
    assert!(frame.completion(0).is_none());
    frame.drawn(view(1).retained, Entity::from_raw_u32(50).unwrap(), 1);
    let result = frame.completion(0).unwrap();
    assert_eq!(result.disposition, FrontierDisposition::Culled);
    assert_eq!((result.source_samples, result.selected_items), (2, 0));
}

#[test]
fn distinct_camera_generations_have_independent_completion_targets() {
    let backend = DrawingBackend::default();
    let mut frame = frame(&backend, false, 2, 1);
    let mut next = super::super::worker::test_dataset(backend.budget(), false);
    Arc::get_mut(&mut next).unwrap().revision.generation = 3;
    frame.entries[0].dataset = next;
    frame.views[0].finished.store(true, Ordering::Release);
    frame.views[1].finished.store(true, Ordering::Release);
    let entity = Entity::from_raw_u32(50).unwrap();
    frame.drawn(view(1).retained, entity, 1);
    assert_eq!(frame.completion(0).unwrap().target.dataset.generation, 3);
    assert!(frame.completion(1).is_none());
    frame.drawn(view(2).retained, entity, 1);
    let old = frame.completion(1).unwrap();
    assert_eq!(old.target.dataset.generation, 1);
    assert_eq!(old.target.view, view(2).identity);
}

#[test]
fn stalled_callbacks_admit_only_two_batches_and_release_their_charges() {
    let backend = DrawingBackend::default();
    let baseline = backend.budget().usage().committed();
    let limiter = limiter(&backend);
    let first = Permit::acquire(&limiter).unwrap();
    let second = Permit::acquire(&limiter).unwrap();
    assert!(Permit::acquire(&limiter).is_none());
    let callback = move || drop(first);
    assert_eq!(limiter.count.load(Ordering::Acquire), 2);
    callback();
    let third = Permit::acquire(&limiter).unwrap();
    drop((second, third, limiter));
    assert_eq!(backend.budget().usage().committed(), baseline);
}

#[test]
fn main_world_view_lifetimes_survive_pauses_but_not_window_replacement() {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::render::MainWorld;
    let backend = DrawingBackend::default();
    backend
        .enable_renderer_visibility(MAX_VIEWS, &[DatasetKind::Cloud])
        .unwrap();
    let mut render = World::new();
    let mut main = MainWorld::default();
    let window = main.spawn((Window::default(), PrimaryWindow)).id();
    let camera = main
        .spawn((
            Camera3d::default(),
            RenderLayers::layer(super::super::super::DRAW_LAYER),
        ))
        .id();
    main.spawn((
        Camera3d::default(),
        RenderTarget::None {
            size: UVec2::splat(32),
        },
        RenderLayers::layer(super::super::super::DRAW_LAYER),
    ));
    main.spawn(Camera3d::default());
    render.insert_resource(main);
    render.init_resource::<ViewRoster>();
    let mut receipts = Receipts::default();
    let sample = |render: &mut World, receipts: &mut Receipts| {
        render.run_system_once(extract_views).unwrap();
        sync_views(receipts, &backend, render.resource::<ViewRoster>()).unwrap();
    };
    sample(&mut render, &mut receipts);
    assert_eq!(receipts.views.len(), 1);
    let original = receipts.views[0].identity;
    render
        .resource_mut::<MainWorld>()
        .get_mut::<Camera>(camera)
        .unwrap()
        .is_active = false;
    render
        .resource_mut::<MainWorld>()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set_physical_resolution(0, 0);
    sample(&mut render, &mut receipts);
    assert_eq!(receipts.views[0].identity, original);
    render
        .resource_mut::<MainWorld>()
        .get_mut::<Camera>(camera)
        .unwrap()
        .is_active = true;
    sample(&mut render, &mut receipts);
    assert_eq!(receipts.views[0].identity, original);
    render.resource_mut::<MainWorld>().despawn(window);
    render
        .resource_mut::<MainWorld>()
        .spawn((Window::default(), PrimaryWindow));
    sample(&mut render, &mut receipts);
    assert_ne!(receipts.views[0].identity, original);
    render.resource_mut::<MainWorld>().despawn(camera);
    sample(&mut render, &mut receipts);
    assert!(receipts.views.is_empty());
}

#[test]
fn embedded_chase_view_survives_resize_and_excludes_arbitrary_texture_cameras() {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::render::MainWorld;
    let backend = DrawingBackend::default();
    backend
        .enable_renderer_visibility(MAX_VIEWS, &[DatasetKind::Cloud])
        .unwrap();
    let mut render = World::new();
    let mut main = MainWorld::default();
    let handle = ManualTextureViewHandle(41);
    let camera = main
        .spawn((
            Camera3d::default(),
            ChaseCamera::default(),
            RenderTarget::TextureView(handle),
            RenderLayers::layer(super::super::super::DRAW_LAYER),
        ))
        .id();
    main.spawn((
        Camera3d::default(),
        RenderTarget::TextureView(handle),
        RenderLayers::layer(super::super::super::DRAW_LAYER),
    ));
    render.insert_resource(main);
    render.init_resource::<ViewRoster>();
    let mut receipts = Receipts::default();
    let sample = |render: &mut World, receipts: &mut Receipts| {
        render.run_system_once(extract_views).unwrap();
        sync_views(receipts, &backend, render.resource::<ViewRoster>()).unwrap();
    };
    sample(&mut render, &mut receipts);
    assert_eq!(receipts.views.len(), 1);
    let original = receipts.views[0].identity;
    render
        .resource_mut::<MainWorld>()
        .get_mut::<Camera>(camera)
        .unwrap()
        .is_active = false;
    sample(&mut render, &mut receipts);
    assert_eq!(receipts.views[0].identity, original);
    render
        .resource_mut::<MainWorld>()
        .get_mut::<Camera>(camera)
        .unwrap()
        .is_active = true;
    sample(&mut render, &mut receipts);
    assert_eq!(receipts.views[0].identity, original);
    render
        .resource_mut::<MainWorld>()
        .entity_mut(camera)
        .insert(RenderTarget::TextureView(ManualTextureViewHandle(42)));
    sample(&mut render, &mut receipts);
    assert_ne!(receipts.views[0].identity, original);
    render.resource_mut::<MainWorld>().despawn(camera);
    sample(&mut render, &mut receipts);
    assert!(receipts.views.is_empty());
}

#[test]
fn a_frame_with_only_compact_instance_data_still_completes() {
    let backend = DrawingBackend::default();
    let mut frame = frame(&backend, false, 1, 0);
    frame.entries.clear();
    let page = Entity::from_raw_u32(90).unwrap();
    frame.octrees.push(OctreeEntry {
        identity: Arc::new(super::super::octree::Identity {
            key: gearbox_api::wire::draw_data::DataKey::new("receipt-test", "volume").unwrap(),
            revision: gearbox_api::wire::draw_data::DatasetRevision { incarnation: 1, generation: 1 },
            leaves: 512,
            cells: 480,
            pages: 1,
        }),
        view: 0,
        expected: 1,
        drawn: AtomicUsize::new(0),
    });
    frame.octree_pages.push(OctreeMark { view: 0, entity: page, entry: 0, drawn: AtomicBool::new(false) });
    assert!(!frame.completes());
    frame.drawn_octree(view(1).retained, page);
    frame.views[0].finished.store(true, Ordering::Release);
    assert!(frame.completes(), "an octree-path draw alone must publish its receipt");
    assert_eq!(frame.octree_completion(0).unwrap().selected_items, 480);
}
