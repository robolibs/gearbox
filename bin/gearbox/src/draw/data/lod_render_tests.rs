use super::*;
use bevy::ecs::system::SystemState;
use std::collections::BTreeSet;

#[test]
fn admission_keeps_placed_previous_pages_until_the_new_generation_is_placed() {
    let budget = CpuBudget::new(1 << 20);
    let key = lod::PageKey {
        incarnation: 1,
        scan: 0,
        page: 0,
    };
    let other = lod::PageKey { page: 1, ..key };
    let old = Entity::from_raw_u32(10).unwrap();
    let new = Entity::from_raw_u32(20).unwrap();
    let mut pages = List::new(&budget, 4, MemoryClass::Scratch).unwrap();
    pages.push((key, 1, old)).unwrap();
    pages.push((other, 1, old)).unwrap();
    newest_pages(&mut pages);
    assert_eq!(pages.as_slice(), &[(key, 1, old), (other, 1, old)]);
    pages.push((key, 2, new)).unwrap();
    newest_pages(&mut pages);
    assert_eq!(pages.as_slice(), &[(key, 2, new), (other, 1, old)]);
    pages.push((key, 1, old)).unwrap();
    newest_pages(&mut pages);
    assert_eq!(pages.as_slice(), &[(key, 2, new), (other, 1, old)]);
    drop(pages);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn duplicate_or_out_of_range_ordinals_cannot_become_direct_draw_ranges() {
    assert_eq!(
        validate_ordinals(&mut [0, 0, 2], 3),
        Err(Issue::InvalidSelection)
    );
    assert_eq!(
        validate_ordinals(&mut [0, 3], 3),
        Err(Issue::InvalidSelection)
    );
    let mut selected = [2, 0, 1];
    assert_eq!(validate_ordinals(&mut selected, 3), Ok(()));
    assert_eq!(selected, [0, 1, 2]);
}

fn scene(budget: &CpuBudget, count: usize) -> (World, Arc<DatasetSet>) {
    let points: Vec<_> = (0..count)
        .map(|i| {
            [
                (i % 31) as f64 * 0.025 - 0.4,
                (i / 31 % 31) as f64 * 0.025 - 0.4 + (i / 961) as f64 * 0.0001,
                0.5,
            ]
        })
        .collect();
    let data = super::super::super::worker::test_dataset_points(budget, &points);
    let datasets = DatasetSet::select(budget, None, &[data.clone()], |_| true, |_| true).unwrap();
    let mut world = World::new();
    for page in &data.pages {
        let entity = world.spawn_empty().id();
        world.entity_mut(entity).insert((
            MainEntity::from(entity),
            Node::for_tests(budget, data.clone(), page.clone()),
        ));
    }
    (world, datasets)
}

fn prepare(
    budget: &CpuBudget,
    world: &mut World,
    datasets: Arc<DatasetSet>,
    points: u32,
    clip: Mat4,
    old: Option<&Bundle>,
) -> Result<Bundle, Issue> {
    prepare_tier(budget, world, datasets, points, clip, Tier::Full, old)
}

fn prepare_tier(
    budget: &CpuBudget,
    world: &mut World,
    datasets: Arc<DatasetSet>,
    points: u32,
    clip: Mat4,
    tier: Tier,
    old: Option<&Bundle>,
) -> Result<Bundle, Issue> {
    prepare_admitted(budget, world, datasets, points, clip, |_| tier, old)
}

fn prepare_admitted(
    budget: &CpuBudget,
    world: &mut World,
    datasets: Arc<DatasetSet>,
    points: u32,
    clip: Mat4,
    admission: impl Fn(lod::PageKey) -> Tier,
    old: Option<&Bundle>,
) -> Result<Bundle, Issue> {
    let mut state = SystemState::<Query<(Entity, &MainEntity, &Node)>>::new(world);
    let nodes = state.get(world).unwrap();
    let mut placements = List::new(budget, nodes.iter().count(), MemoryClass::Metadata)
        .map_err(|_| Issue::CpuBudget)?;
    for (entity, _, node) in &nodes {
        let address = node.page.appearance.geometry.descriptor().address;
        placements
            .push(Placement {
                entity,
                key: lod::PageKey {
                    incarnation: node.dataset.revision.incarnation,
                    scan: address.scan_key,
                    page: address.page_id,
                },
                world: node.world_from_local,
                glyph: lod::Glyph::default(),
            })
            .unwrap();
    }
    placements.sort_unstable_by_key(|p| p.key);
    build_admitted(
        budget,
        datasets,
        Request {
            view: lod::View {
                clip_from_world: clip.as_dmat4(),
                viewport: [800, 600],
                error_px: 0.01,
                hysteresis: 0.15,
            },
            placements,
        },
        &nodes,
        lod::Limits {
            points,
            bytes: u64::from(points) * 4,
            bytes_per_point: 4,
        },
        admission,
        old,
    )
}

#[test]
fn mixed_page_cuts_preserve_source_identity_and_global_limits_through_tier_changes() {
    let budget = CpuBudget::new(32 << 20);
    let count = 8194;
    let points: Vec<_> = (0..count)
        .map(|i| {
            [
                (i % 61) as f64 * 0.01 - 0.3,
                (i / 61 % 61) as f64 * 0.01 - 0.3,
                0.4 + (i / 3721) as f64 * 0.01,
            ]
        })
        .collect();
    let data = super::super::super::worker::test_dataset_paged(&budget, &points, count / 2);
    let datasets = DatasetSet::select(&budget, None, &[data.clone()], |_| true, |_| true).unwrap();
    let mut world = World::new();
    for page in &data.pages {
        let entity = world.spawn_empty().id();
        world.entity_mut(entity).insert((
            MainEntity::from(entity),
            Node::for_tests(&budget, data.clone(), page.clone()),
        ));
    }
    let mut old = None;
    for full_page in [0, 1, 0] {
        let cut = prepare_admitted(
            &budget,
            &mut world,
            datasets.clone(),
            5000,
            Mat4::IDENTITY,
            |key| {
                if key.page == full_page {
                    Tier::Full
                } else {
                    Tier::Coarse
                }
            },
            old.as_ref(),
        )
        .unwrap();
        assert!(!cut.full() && !cut.coarse());
        assert_eq!(cut.pages.len(), 2);
        assert_eq!(cut.history.statistics.valid, count as u64);
        assert_eq!(cut.history.statistics.represented, count as u64);
        assert!(cut.history.statistics.selected <= 5000);
        assert!(cut.history.statistics.pending_branches > 0);
        let mut ids = BTreeSet::new();
        for page in &cut.pages {
            let geometry = page.geometry.upgrade().unwrap();
            let values: Vec<_> = page
                .values()
                .map_or_else(|| page.range.clone().collect(), |values| values.to_vec());
            for index in values {
                let ordinal = match page.tier {
                    Tier::Full => index,
                    Tier::Coarse => geometry.coarse.ordinals.values[index as usize],
                };
                assert!(ids.insert(geometry.source_id(ordinal as usize).unwrap()));
            }
            assert_eq!(
                page.tier,
                if page.key.page == full_page {
                    Tier::Full
                } else {
                    Tier::Coarse
                }
            );
        }
        assert_eq!(ids.len() as u64, cut.history.statistics.selected);
        assert!(!cut.ready(&[]));
        old = Some(cut);
    }
    drop((old, world, datasets, data));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn coarse_cut_addresses_compact_records_and_keeps_original_source_statistics() {
    let budget = CpuBudget::new(16 << 20);
    let (mut world, datasets) = scene(&budget, 4097);
    let coarse = prepare_tier(
        &budget,
        &mut world,
        datasets.clone(),
        4097,
        Mat4::IDENTITY,
        Tier::Coarse,
        None,
    )
    .unwrap();
    assert!(coarse.coarse());
    assert!(!coarse.full());
    assert_eq!(coarse.history.statistics.represented, 4097);
    assert!(coarse.history.statistics.pending_branches > 0);
    assert!(coarse.history.pieces.iter().all(|piece| !piece.full));
    let page = &coarse.pages[0];
    let geometry = page.geometry.upgrade().unwrap();
    assert_eq!(
        page.selected as usize,
        geometry.coarse.positions.values.len()
    );
    assert_eq!(page.range, 0..page.selected as u32);
    assert!(!page.indexed());
    let source_ids: BTreeSet<_> = coarse
        .history
        .pieces
        .iter()
        .map(|piece| {
            let node = geometry.hierarchy.nodes.values[piece.node as usize];
            geometry.source_id(node.representative as usize).unwrap()
        })
        .collect();
    assert_eq!(
        source_ids,
        geometry.coarse.source_ids.values.iter().copied().collect()
    );
    let full = prepare(&budget, &mut world, datasets, 4097, Mat4::IDENTITY, None).unwrap();
    let coarse_bytes = demand_bytes(|| coarse.pages.iter());
    let full_bytes = demand_bytes(|| full.pages.iter());
    assert!(coarse_bytes < full_bytes);
    assert_eq!(
        demand_bytes(|| full.pages.iter().chain(full.pages.iter())),
        full_bytes + 512
    );
    assert!(demand_bytes(|| full.pages.iter().chain(coarse.pages.iter())) > full_bytes);
    drop((geometry, full, coarse, world));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn full_source_and_single_representative_draw_without_an_index_buffer() {
    let budget = CpuBudget::new(16 << 20);
    let (mut world, datasets) = scene(&budget, 1025);
    let full = prepare(
        &budget,
        &mut world,
        datasets.clone(),
        1025,
        Mat4::IDENTITY,
        None,
    )
    .unwrap();
    assert_eq!(full.pages.len(), 1);
    assert_eq!(full.pages[0].range, 0..1025);
    assert!(!full.pages[0].indexed());
    assert!(full.pages[0].ready());
    assert_eq!(full.pages[0].selected, 1025);
    assert!(full.history.indices.is_empty());
    assert_eq!(full.history.indices.capacity(), 0);
    let coarse = prepare(&budget, &mut world, datasets, 1, Mat4::IDENTITY, None).unwrap();
    assert_eq!(coarse.pages[0].range.len(), 1);
    assert!(!coarse.pages[0].indexed());
    assert_eq!(coarse.pages[0].selected, 1);
    drop((full, coarse, world));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn noncontiguous_cut_materializes_exact_bounded_ordinals_and_waits_for_upload() {
    let budget = CpuBudget::new(16 << 20);
    let (mut world, datasets) = scene(&budget, 4097);
    let selected = prepare(&budget, &mut world, datasets, 16, Mat4::IDENTITY, None).unwrap();
    assert_eq!(selected.history.statistics.selected, 16);
    let page = &selected.pages[0];
    assert!(page.indexed());
    assert!(!page.ready());
    assert_eq!(page.range, 0..16);
    assert_eq!(page.selected, 16);
    let ordinals = page.values().unwrap();
    assert!(ordinals.windows(2).all(|pair| pair[0] < pair[1]));
    let geometry = page.geometry.upgrade().unwrap();
    let ids: BTreeSet<_> = ordinals
        .iter()
        .map(|ordinal| geometry.source_id(*ordinal as usize).unwrap())
        .collect();
    assert_eq!(ids.len(), 16);
    assert!(ordinals.iter().all(|ordinal| *ordinal < 4097));
    assert!(selected.history.indices.is_empty());
    drop((geometry, selected, world));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn culled_cut_keeps_dataset_identity_without_draws_or_false_selected_counts() {
    let budget = CpuBudget::new(16 << 20);
    let (mut world, datasets) = scene(&budget, 1025);
    let bundle = prepare(
        &budget,
        &mut world,
        datasets.clone(),
        16,
        Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)),
        None,
    )
    .unwrap();
    assert!(bundle.pages.is_empty());
    assert_eq!(bundle.history.statistics.selected, 0);
    assert_eq!(bundle.history.statistics.culled, 1025);
    let key = RetainedViewEntity::new(Entity::from_raw_u32(999).unwrap().into(), None, 0);
    let identity = ViewIdentity { id: 1, epoch: 2 };
    let mut views = Views::default();
    views
        .states
        .insert(
            &budget,
            key,
            State {
                identity,
                active: Some(bundle),
                pending: None,
                job: None,
            },
        )
        .unwrap();
    assert_eq!(
        views.counts(key, identity, &datasets.datasets.read()[0]),
        Some((0, 0))
    );
    assert_eq!(
        views.counts(
            key,
            ViewIdentity {
                epoch: 3,
                ..identity
            },
            &datasets.datasets.read()[0]
        ),
        None
    );
    drop((views, datasets, world));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn denied_replacement_leaves_previous_draw_ranges_and_source_ownership_unchanged() {
    let budget = CpuBudget::new(16 << 20);
    let (mut world, datasets) = scene(&budget, 1025);
    let old = prepare(
        &budget,
        &mut world,
        datasets.clone(),
        1025,
        Mat4::IDENTITY,
        None,
    )
    .unwrap();
    let baseline = budget.usage().committed();
    let pressure = budget.reserve(budget.usage().limit - baseline).unwrap();
    assert!(
        prepare(
            &budget,
            &mut world,
            datasets.clone(),
            8,
            Mat4::IDENTITY,
            Some(&old)
        )
        .is_err()
    );
    assert_eq!(old.pages[0].range, 0..1025);
    assert!(old.pages[0].ready());
    drop(pressure);
    assert_eq!(budget.usage().committed(), baseline);
    let replacement =
        prepare(&budget, &mut world, datasets, 8, Mat4::IDENTITY, Some(&old)).unwrap();
    assert_eq!(
        replacement
            .pages
            .iter()
            .map(|p| p.range.len())
            .sum::<usize>(),
        8
    );
    assert_eq!(replacement.history.statistics.selected, 8);
    drop((replacement, old, world));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn direct_ranges_preserve_nonzero_original_ordinals() {
    assert_eq!(direct_range(&[]), None);
    assert_eq!(direct_range(&[42]), Some(42..43));
    assert_eq!(direct_range(&[43, 41, 42]), Some(41..44));
    assert_eq!(direct_range(&[41, 43]), None);
}

#[test]
fn residency_demand_unions_active_pending_and_other_views_without_duplicates_in_membership() {
    let budget = CpuBudget::new(16 << 20);
    let (mut world, datasets) = scene(&budget, 1025);
    let active = prepare(
        &budget,
        &mut world,
        datasets.clone(),
        1,
        Mat4::IDENTITY,
        None,
    )
    .unwrap();
    assert!(!active.ready(&[]));
    let mut pending = prepare(
        &budget,
        &mut world,
        datasets.clone(),
        16,
        Mat4::IDENTITY,
        None,
    )
    .unwrap();
    let mut other = prepare(
        &budget,
        &mut world,
        datasets.clone(),
        1,
        Mat4::IDENTITY,
        None,
    )
    .unwrap();
    let a = active.pages[0].entity;
    let b = Entity::from_raw_u32(1234).unwrap();
    let c = Entity::from_raw_u32(4321).unwrap();
    pending.pages[0].entity = b;
    other.pages[0].entity = c;
    let mut views = Views::default();
    let first = RetainedViewEntity::new(a.into(), None, 0);
    let second = RetainedViewEntity::new(c.into(), None, 0);
    views
        .states
        .insert(
            &budget,
            first,
            State {
                identity: ViewIdentity { id: 1, epoch: 1 },
                active: Some(active),
                pending: Some(pending),
                job: None,
            },
        )
        .unwrap();
    views
        .states
        .insert(
            &budget,
            second,
            State {
                identity: ViewIdentity { id: 2, epoch: 1 },
                active: Some(other),
                pending: None,
                job: None,
            },
        )
        .unwrap();
    assert_eq!(views.needed().collect::<BTreeSet<_>>(), [a, b, c].into());
    #[cfg(feature = "draw-probes")]
    assert_eq!(views.demanded_position_bytes(), 1025 * 16);
    assert!(views.needs(a) && views.needs(b) && views.needs(c));
    views.states.get_mut(&first).unwrap().active = None;
    assert!(!views.needs(a));
    assert!(views.needs(b) && views.needs(c));
    let state = views.states.get_mut(&first).unwrap();
    state.active = state.pending.take();
    assert!(views.needs(b) && views.needs(c));
    views.cancel_unpublished(first);
    assert!(views.needs(b));
    let state = views.states.get_mut(&first).unwrap();
    state.pending = state.active.take();
    let pressure = budget
        .reserve(budget.usage().limit - budget.usage().committed())
        .unwrap();
    views.cancel_unpublished(first);
    assert!(!views.needs(b));
    assert!(views.needs(c));
    drop(pressure);
    drop((views, world, datasets));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn hollow_pages_are_wanted_at_full_detail_but_drawn_from_coarse_records() {
    let budget = CpuBudget::new(1 << 20);
    let key = |page| lod::PageKey {
        incarnation: 3,
        scan: 0,
        page,
    };
    let mut pages = List::new(&budget, 3, MemoryClass::Metadata).unwrap();
    for (page, full) in [(0, true), (1, true), (2, false)] {
        pages
            .push(residency::Page {
                key: key(page),
                coarse_bytes: 512,
                full_bytes: 1024,
                full,
            })
            .unwrap();
    }
    let mut hollow = List::new(&budget, 1, MemoryClass::Scratch).unwrap();
    hollow.push(key(1)).unwrap();
    let admission = Admission {
        plan: residency::Plan { pages, bytes: 2048 },
        hollow,
        stamp: 1,
        wanted: 2,
    };
    assert_eq!(admission.tier(key(0)), Tier::Full);
    assert_eq!(admission.tier(key(1)), Tier::Coarse);
    assert_eq!(admission.tier(key(2)), Tier::Coarse);
    assert_eq!(admission.wanted().collect::<Vec<_>>(), vec![key(0), key(1)]);
}
