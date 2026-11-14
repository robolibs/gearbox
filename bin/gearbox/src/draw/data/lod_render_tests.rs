use super::*;
use bevy::ecs::system::SystemState;

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

fn bundle(
    budget: &CpuBudget,
    world: &mut World,
    datasets: Arc<DatasetSet>,
    clip: Mat4,
    tier: Tier,
) -> Result<Bundle, Issue> {
    let mut state = SystemState::<Query<(Entity, &MainEntity, &Node)>>::new(world);
    let nodes = state.get(world).unwrap();
    let view = lod::View {
        clip_from_world: clip.as_dmat4(),
        viewport: [800, 600],
        error_px: 1.5,
        hysteresis: 0.15,
    };
    let mut placements = List::new(budget, nodes.iter().count(), MemoryClass::Metadata).unwrap();
    for (entity, _, node) in &nodes {
        placements
            .push(Placement {
                entity,
                key: node_key(node),
                world: node.world_from_local,
                glyph: lod::Glyph::default(),
            })
            .unwrap();
    }
    placements.sort_unstable_by_key(|p| p.key);
    build(budget, datasets, Request { view, placements }, &nodes, |_| tier)
}

#[test]
fn full_pages_take_lod_slots_while_coarse_pages_draw_every_accepted_record() {
    let budget = CpuBudget::new(64 << 20);
    let (mut world, datasets) = scene(&budget, 4000);
    let full = bundle(&budget, &mut world, datasets.clone(), Mat4::IDENTITY, Tier::Full).unwrap();
    assert!(!full.pages.is_empty());
    for (slot, page) in full.pages.iter().enumerate() {
        assert_eq!(page.tier, Tier::Full);
        assert_eq!(page.slot, Some(slot as u32));
        assert!(page.indexed() && !page.ready());
        assert_eq!((page.range.clone(), page.selected), (0..0, 0));
    }
    assert_eq!(full.slots(), full.pages.len() as u32);
    let coarse = bundle(&budget, &mut world, datasets, Mat4::IDENTITY, Tier::Coarse).unwrap();
    for page in coarse.pages.iter() {
        let geometry = page.geometry.upgrade().unwrap();
        let records = geometry.coarse.ordinals.values.len();
        assert_eq!(page.tier, Tier::Coarse);
        assert_eq!((page.slot, page.range.clone()), (None, 0..records as u32));
        assert_eq!(page.selected, records as u64);
        assert!(!page.indexed() && page.ready());
    }
    assert_eq!(coarse.slots(), 0);
}

#[test]
fn pages_outside_the_view_are_left_out_of_its_page_set() {
    let budget = CpuBudget::new(64 << 20);
    let (mut world, datasets) = scene(&budget, 4000);
    let away = Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0));
    let culled = bundle(&budget, &mut world, datasets, away, Tier::Full).unwrap();
    assert!(culled.pages.is_empty());
    assert_eq!(culled.slots(), 0);
}

#[test]
fn lod_headroom_is_what_the_view_buffers_allocate_per_page() {
    use super::super::gpu::{HEAD_STRIDE, TABLE_BYTES};
    let reserve = residency::headroom(1, 3, 0, 0, 0, 0);
    assert_eq!(reserve, 4 * (HEAD_STRIDE + TABLE_BYTES) + 4096);
}
