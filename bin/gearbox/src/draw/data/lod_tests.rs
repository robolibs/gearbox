use std::cell::Cell;
use std::collections::BTreeSet;

use bevy::math::DVec3;

use super::*;
use crate::draw::data::tests::{fixture, stored};

#[test]
fn selection_history_retains_identity_without_pinning_source_payloads() {
    let budget = CpuBudget::new(16 << 20);
    let geometry = geometry(&budget, 4097, 0, 0.01);
    let weak = Arc::downgrade(&geometry);
    let cut = choose(&budget, &[input(&geometry, 1, 0, 0)], 16);
    let before = budget.usage().committed();
    drop(geometry);
    assert!(weak.upgrade().is_none());
    assert!(cut.pages[0].geometry.upgrade().is_none());
    assert!(budget.usage().committed() < before);
    assert!(budget.usage().committed() >= size_of::<Geometry>() as u64);
    drop((cut, weak));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn metric_glyph_does_not_expand_unchanged_shader_depth() {
    let budget = CpuBudget::new(16 << 20);
    let source = geometry(&budget, 1, 0, 0.0);
    for z in [-1.0, 1.0] {
        let mut sample = input(&source, 1, 0, 0);
        sample.world_from_local *= DMat4::from_translation(DVec3::new(0.0, 0.0, z));
        sample.glyph.metres = 100.0;
        let cut = choose(&budget, &[sample], 1);
        assert_eq!(cut.statistics.selected, 0);
        assert_eq!(cut.statistics.culled, 1);
    }
}

#[test]
fn metadata_selects_and_retains_fallback_after_source_payload_is_evicted() {
    let budget = CpuBudget::new(16 << 20);
    let source = geometry(&budget, 4097, 7000, 0.01);
    let expected = choose(&budget, &[input(&source, 1, 0, 0)], 16);
    let expected_indices = expected.indices.to_vec();
    let expected_statistics = expected.statistics;
    let metadata = source.metadata.clone();
    let geometry_identity = Arc::downgrade(&source);
    let payload = Arc::downgrade(source.source().unwrap());
    let source_ids: Vec<_> = (0..4097)
        .map(|index| source.source_id(index).unwrap())
        .collect();
    let before = budget.usage().committed();
    drop((expected, source));
    assert!(payload.upgrade().is_none());
    assert!(geometry_identity.upgrade().is_none());
    assert!(budget.usage().committed() < before);
    let inputs = [Input {
        key: PageKey {
            incarnation: 1,
            scan: 0,
            page: 0,
        },
        geometry: geometry_identity.clone(),
        metadata: &metadata,
        world_from_local: DMat4::from_translation(DVec3::from_array(metadata.rebase_m)),
        glyph: Glyph::default(),
    }];
    let fallback = select(
        &budget,
        &inputs,
        view(0.01),
        limits(16),
        None,
        |sample| sample.node == 0 && !sample.full,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(fallback.statistics.selected, 1);
    assert_eq!(fallback.statistics.represented, 4097);
    assert!(fallback.statistics.pending_branches > 0);
    let restored = select(
        &budget,
        &inputs,
        view(0.01),
        limits(16),
        Some(&fallback),
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(restored.indices.as_slice(), expected_indices);
    assert_eq!(restored.statistics, expected_statistics);
    for &ordinal in &restored.indices {
        assert_eq!(source_ids[ordinal as usize], 7000 + u64::from(ordinal));
    }
    drop((restored, fallback, inputs));
    drop((metadata, geometry_identity, payload));
    assert_eq!(budget.usage().committed(), 0);
}

fn geometry(budget: &CpuBudget, count: usize, base: u64, scale: f64) -> Arc<Geometry> {
    let points: Vec<_> = (0..count)
        .map(|i| {
            [
                ((i % 17) as f64 - 8.0) * scale,
                ((i / 17 % 17) as f64 - 8.0) * scale,
                0.5,
            ]
        })
        .collect();
    let (schema, mut page) = fixture(&points, vec![]);
    page.columns[1].values = (0..count)
        .flat_map(|i| (base + i as u64).to_le_bytes())
        .collect();
    Geometry::prepare(budget, &schema, stored(budget, &schema, &page, [0.0; 3])).unwrap()
}

fn input(geometry: &Arc<Geometry>, incarnation: u64, scan: u64, page: u64) -> Input<'_> {
    Input {
        key: PageKey {
            incarnation,
            scan,
            page,
        },
        geometry: Arc::downgrade(geometry),
        metadata: &geometry.metadata,
        world_from_local: DMat4::from_translation(DVec3::from_array(geometry.rebase_m)),
        glyph: Glyph::default(),
    }
}

fn unique_geometry(budget: &CpuBudget, count: usize, base: u64) -> Arc<Geometry> {
    let side = (count as f64).sqrt().ceil() as usize;
    let step = 1.6 / side.max(1) as f64;
    let points: Vec<_> = (0..count)
        .map(|i| {
            [
                -0.8 + (i % side) as f64 * step,
                -0.8 + (i / side) as f64 * step,
                0.5,
            ]
        })
        .collect();
    let (schema, mut page) = fixture(&points, vec![]);
    page.columns[1].values = (0..count)
        .flat_map(|i| (base + i as u64).to_le_bytes())
        .collect();
    Geometry::prepare(budget, &schema, stored(budget, &schema, &page, [0.0; 3])).unwrap()
}

fn view(error_px: f64) -> View {
    View {
        clip_from_world: DMat4::IDENTITY,
        viewport: [100, 100],
        error_px,
        hysteresis: 0.2,
    }
}

fn limits(points: u32) -> Limits {
    Limits {
        points,
        bytes: u64::from(points) * 52,
        bytes_per_point: 52,
    }
}

fn choose(budget: &CpuBudget, inputs: &[Input<'_>], points: u32) -> Cut {
    select(
        budget,
        inputs,
        view(0.01),
        limits(points),
        None,
        |_| true,
        || Ok(()),
    )
    .unwrap()
}

#[test]
fn partial_clear_recounts_pending_budget_and_culled_sources_without_allocating() {
    let budget = CpuBudget::new(16 << 20);
    let a = geometry(&budget, 1025, 0, 0.04);
    let b = geometry(&budget, 513, 2000, 0.04);
    let c = geometry(&budget, 17, 4000, 0.04);
    let mut hidden = input(&c, 3, 0, 0);
    hidden.world_from_local = DMat4::from_translation(DVec3::new(20.0, 0.0, 0.0));
    let mut cut = select(
        &budget,
        &[input(&a, 1, 0, 0), input(&b, 2, 0, 0), hidden],
        view(0.01),
        limits(16),
        None,
        |samples| samples.page.incarnation != 2 || (samples.node == 0 && !samples.full),
        || Ok(()),
    )
    .unwrap();
    assert!(cut.statistics.pending_branches > 0);
    assert!(cut.statistics.budget_limited_branches > 0);
    let pending = cut.statistics.pending_branches;
    let retained_ids: Vec<_> = ids(&cut).into_iter().filter(|id| id.0 != 1).collect();
    let pressure = budget
        .reserve(budget.usage().limit - budget.usage().committed())
        .unwrap();
    cut.retain(|incarnation| incarnation != 1);
    assert_eq!(ids(&cut), retained_ids);
    assert_eq!(cut.statistics.valid, 530);
    assert_eq!(cut.statistics.represented, 513);
    assert_eq!(cut.statistics.culled, 17);
    assert_eq!(cut.statistics.pending_branches, pending);
    assert_eq!(cut.statistics.budget_limited_branches, 0);
    assert_eq!(cut.pieces[0].page, 0);
    cut.indices = List::default();
    cut.retain(|incarnation| incarnation == 3);
    assert_eq!(
        cut.statistics,
        Statistics {
            valid: 17,
            culled: 17,
            ..Default::default()
        }
    );
    assert!(cut.pieces.is_empty());
    cut.retain(|_| false);
    assert_eq!(cut.statistics, Statistics::default());
    assert_eq!(cut.pages.capacity(), 0);
    assert_eq!(cut.pieces.capacity(), 0);
    drop((pressure, cut, a, b, c));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn partial_clear_preserves_cross_page_dataset_roots_and_hysteresis_history() {
    let budget = CpuBudget::new(16 << 20);
    let geometry = geometry(&budget, 129, 0, 0.04);
    let inputs = [
        input(&geometry, 1, 0, 0),
        input(&geometry, 2, 0, 0),
        input(&geometry, 2, 0, 1),
    ];
    let mut cut = choose(&budget, &inputs, 2);
    assert_eq!(cut.statistics.represented, 387);
    let expected = ids(&cut)
        .into_iter()
        .filter(|id| id.0 == 2)
        .collect::<Vec<_>>();
    cut.retain(|incarnation| incarnation == 2);
    assert_eq!(ids(&cut), expected);
    assert_eq!(cut.statistics.valid, 258);
    assert_eq!(cut.statistics.represented, 258);
    assert_eq!(cut.statistics.selected, 1);
    assert_eq!(cut.statistics.budget_limited_branches, 1);
    assert_eq!(cut.pieces[0].page, 0);
    assert!(cut.pieces[0].dataset_root);
    assert!(!cut.expanded_dataset(&inputs[1]));
    let replacement = select(
        &budget,
        &inputs[1..],
        view(0.01),
        limits(16),
        Some(&cut),
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert!(replacement.statistics.selected > 1);
    assert!(replacement.expanded_dataset(&inputs[1]));
}

fn ids(cut: &Cut) -> Vec<(u64, u64, u64)> {
    let mut result = Vec::new();
    for page in &cut.pages {
        for &ordinal in &cut.indices[page.first as usize..(page.first + page.count) as usize] {
            result.push((
                page.key.incarnation,
                page.key.scan,
                page.geometry
                    .upgrade()
                    .unwrap()
                    .source_id(ordinal as usize)
                    .unwrap(),
            ));
        }
    }
    assert_eq!(
        result.iter().copied().collect::<BTreeSet<_>>().len(),
        result.len()
    );
    assert_eq!(result.len() as u64, cut.statistics.selected);
    assert_eq!(
        cut.statistics.valid,
        cut.statistics.represented + cut.statistics.culled
    );
    result
}

#[test]
fn global_point_and_byte_limits_keep_all_visible_datasets_represented() {
    let budget = CpuBudget::new(16 << 20);
    let a = geometry(&budget, 513, 0, 0.1);
    let b = geometry(&budget, 257, 1000, 0.1);
    let c = geometry(&budget, 513, 0, 0.1);
    let inputs = [input(&a, 1, 0, 0), input(&b, 1, 0, 1), input(&c, 2, 0, 0)];
    for cap in [2, 3, 5, 64, 255, 256, 511, 1000, 1283] {
        let cut = choose(&budget, &inputs, cap);
        let selected = ids(&cut);
        assert!(selected.len() <= cap as usize);
        assert_eq!(
            selected.iter().map(|id| id.0).collect::<BTreeSet<_>>(),
            BTreeSet::from([1, 2])
        );
        assert_eq!(cut.statistics.represented, 1283);
    }
    let byte_limited = select(
        &budget,
        &inputs,
        view(0.01),
        Limits {
            points: 1283,
            bytes: 3 * 52 + 51,
            bytes_per_point: 52,
        },
        None,
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert!(ids(&byte_limited).len() <= 3);
    assert!(matches!(
        select(
            &budget,
            &inputs,
            view(0.01),
            limits(1),
            None,
            |_| true,
            || Ok(())
        ),
        Err(Error::PointBudget {
            minimum: 2,
            available: 1
        })
    ));
}

#[test]
fn dataset_roots_cover_many_pages_without_admitting_each_page_as_a_point() {
    let budget = CpuBudget::new(16 << 20);
    let geometry: Vec<_> = (0..16)
        .map(|i| geometry(&budget, 300, i * 1000, 0.1))
        .collect();
    let inputs: Vec<_> = geometry
        .iter()
        .enumerate()
        .map(|(i, g)| input(g, 1, 0, i as u64))
        .collect();
    let cut = choose(&budget, &inputs, 1);
    assert_eq!(ids(&cut), vec![(1, 0, 0)]);
    assert_eq!(cut.statistics.represented, 4800);
    assert!(cut.pieces[0].dataset_root);
    assert_eq!(cut.statistics.budget_limited_branches, 1);
}

#[test]
fn full_budget_emits_each_original_source_once_and_input_order_is_irrelevant() {
    let budget = CpuBudget::new(16 << 20);
    let a = unique_geometry(&budget, 513, 0);
    let b = unique_geometry(&budget, 301, 1000);
    let first = choose(&budget, &[input(&a, 1, 0, 0), input(&b, 1, 0, 1)], 1000);
    let second = choose(&budget, &[input(&b, 1, 0, 1), input(&a, 1, 0, 0)], 1000);
    assert_eq!(ids(&first), ids(&second));
    assert_eq!(first.statistics.selected, 814);
    assert_eq!(first.statistics.max_error_px, 0.0);
    let expected: BTreeSet<_> = (0..513).chain(1000..1301).map(|id| (1, 0, id)).collect();
    assert_eq!(ids(&first).into_iter().collect::<BTreeSet<_>>(), expected);
}

#[test]
fn leaf_budget_uses_nested_intermediate_prefixes_without_losing_source_coverage() {
    let budget = CpuBudget::new(16 << 20);
    for count in [3, 5, 129, 193, 255, 256] {
        let geometry = unique_geometry(&budget, count, 1000);
        let inputs = [input(&geometry, 1, 0, 0)];
        let mut previous = BTreeSet::new();
        for cap in 1..=count as u32 {
            let cut = choose(&budget, &inputs, cap);
            let selected = if cap as usize == count {
                cap
            } else {
                1 << cap.ilog2()
            };
            assert_eq!(cut.statistics.selected, u64::from(selected));
            assert_eq!(cut.statistics.represented, count as u64);
            assert_eq!(
                cut.indices.as_slice(),
                &geometry.hierarchy.refinement.order.values[..selected as usize]
            );
            let current: BTreeSet<_> = cut.indices.iter().copied().collect();
            assert_eq!(current.len(), selected as usize);
            assert!(previous.is_subset(&current));
            previous = current;
            assert_eq!(cut.pieces[0].full, selected as usize == count);
        }
        let fallback = select(
            &budget,
            &inputs,
            view(0.01),
            limits(count as u32),
            None,
            |samples| !samples.full,
            || Ok(()),
        )
        .unwrap();
        assert_eq!(fallback.statistics.selected, 1);
        assert_eq!(fallback.statistics.represented, count as u64);
        assert_eq!(fallback.statistics.pending_branches, 1);
    }
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn intermediate_leaf_refinement_preserves_hysteresis_without_crossing_source_identity() {
    let budget = CpuBudget::new(16 << 20);
    let points: Vec<_> = (0..8).map(|i| [-0.7 + i as f64 * 0.2, 0.0, 0.5]).collect();
    let (schema, page) = fixture(&points, vec![]);
    let source = stored(&budget, &schema, &page, [0.0; 3]);
    let geometry = Geometry::prepare(&budget, &schema, source.clone()).unwrap();
    let inputs = [input(&geometry, 1, 0, 0)];
    let coarse = choose(&budget, &inputs, 2);
    let fine = choose(&budget, &inputs, 4);
    assert_eq!(coarse.statistics.selected, 2);
    assert_eq!(fine.statistics.selected, 4);
    let target = view(coarse.statistics.max_error_px);
    for (previous, expected) in [(None, 2), (Some(&coarse), 2), (Some(&fine), 4)] {
        let cut = select(
            &budget,
            &inputs,
            target,
            limits(8),
            previous,
            |_| true,
            || Ok(()),
        )
        .unwrap();
        assert_eq!(cut.statistics.selected, expected);
    }
    let replacement = Geometry::prepare(&budget, &schema, source).unwrap();
    let cut = select(
        &budget,
        &[input(&replacement, 1, 0, 0)],
        target,
        limits(8),
        Some(&fine),
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(cut.statistics.selected, 2);
    drop((cut, replacement, inputs, fine, coarse));
    drop(geometry);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn larger_projected_error_gets_refinement_before_small_distant_clusters() {
    let budget = CpuBudget::new(8 << 20);
    let large = geometry(&budget, 513, 0, 0.1);
    let tiny = geometry(&budget, 513, 0, 0.0001);
    let cut = select(
        &budget,
        &[input(&tiny, 1, 0, 0), input(&large, 2, 0, 0)],
        view(1.0),
        limits(4),
        None,
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(cut.pages[0].count, 1);
    assert_eq!(cut.pages[1].count, 3);
    assert_eq!(ids(&cut).len(), 4);
}

#[test]
fn unavailable_sibling_or_leaf_keeps_the_complete_parent_cut() {
    let budget = CpuBudget::new(8 << 20);
    let geometry = unique_geometry(&budget, 513, 0);
    let inputs = [input(&geometry, 1, 0, 0)];
    let parent = select(
        &budget,
        &inputs,
        view(0.01),
        limits(1000),
        None,
        |samples| samples.node != 1,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(ids(&parent).len(), 1);
    assert_eq!(parent.statistics.represented, 513);
    assert_eq!(parent.statistics.pending_branches, 1);
    let leaves = select(
        &budget,
        &inputs,
        view(0.01),
        limits(1000),
        Some(&parent),
        |samples| !samples.full,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(ids(&leaves).len(), 3);
    assert_eq!(leaves.statistics.represented, 513);
    assert_eq!(leaves.statistics.pending_branches, 3);
    let fine = select(
        &budget,
        &inputs,
        view(0.01),
        limits(1000),
        Some(&leaves),
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(ids(&fine).len(), 513);
    let fallback = select(
        &budget,
        &inputs,
        view(0.01),
        limits(1000),
        Some(&fine),
        |samples| !samples.full,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(ids(&fallback).len(), 3);
    assert_eq!(fallback.statistics.represented, 513);
}

#[test]
fn hysteresis_preserves_both_fine_and_coarse_sides_of_the_dead_band() {
    let budget = CpuBudget::new(4 << 20);
    let geometry = geometry(&budget, 2, 0, 0.04);
    let inputs = [input(&geometry, 1, 0, 0)];
    let fine = select(
        &budget,
        &inputs,
        view(1.0),
        limits(10),
        None,
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(ids(&fine).len(), 2);
    let cold = select(
        &budget,
        &inputs,
        view(2.0),
        limits(10),
        None,
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(ids(&cold).len(), 1);
    let held = select(
        &budget,
        &inputs,
        view(2.0),
        limits(10),
        Some(&fine),
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(ids(&held).len(), 2);
    let coarse = select(
        &budget,
        &inputs,
        view(3.0),
        limits(10),
        Some(&fine),
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(ids(&coarse).len(), 1);
}

#[test]
fn dataset_root_hysteresis_remembers_expansion_to_single_sample_pages() {
    let budget = CpuBudget::new(4 << 20);
    let a = geometry(&budget, 1, 0, 0.0);
    let b = geometry(&budget, 1, 1, 0.0);
    let mut second = input(&b, 1, 0, 1);
    second.world_from_local.w_axis.x += 0.04;
    let inputs = [input(&a, 1, 0, 0), second];
    let fine = select(
        &budget,
        &inputs,
        view(1.0),
        limits(2),
        None,
        |_| true,
        || Ok(()),
    )
    .unwrap();
    let held = select(
        &budget,
        &inputs,
        view(2.0),
        limits(2),
        Some(&fine),
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(ids(&held).len(), 2);
}

#[test]
fn hysteresis_does_not_reuse_replaced_geometry_with_the_same_page_key() {
    let budget = CpuBudget::new(4 << 20);
    let old = geometry(&budget, 2, 0, 0.04);
    let new = geometry(&budget, 2, 0, 0.04);
    let previous = select(
        &budget,
        &[input(&old, 1, 0, 0)],
        view(1.0),
        limits(10),
        None,
        |_| true,
        || Ok(()),
    )
    .unwrap();
    let next = select(
        &budget,
        &[input(&new, 1, 0, 0)],
        view(2.0),
        limits(10),
        Some(&previous),
        |_| true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(ids(&next).len(), 1);
}

#[test]
fn glyph_expansion_prevents_edge_culling_and_near_plane_crossings_stay_conservative() {
    let view = view(1.0);
    let project = |bounds, glyph| view.project(DMat4::IDENTITY, [0.0; 3], bounds, 0.0, glyph);
    let point = [[1.05, 0.0, 0.5]; 2];
    assert!(
        !project(
            point,
            Glyph {
                metres: 0.0,
                pixels: 1.0
            }
        )
        .visible
    );
    assert!(
        project(
            point,
            Glyph {
                metres: 0.0,
                pixels: 3.0
            }
        )
        .visible
    );
    assert!(
        project(
            point,
            Glyph {
                metres: 0.1,
                pixels: 0.0
            }
        )
        .visible
    );
    assert!(
        project(
            point,
            Glyph {
                metres: 0.0,
                pixels: f64::from(f32::MAX)
            }
        )
        .visible
    );
    let crossing = project([[-0.1, -0.1, -0.5], [0.1, 0.1, 0.5]], Glyph::default());
    assert!(crossing.visible && crossing.error().is_infinite());
    assert!(!project([[-0.1, -0.1, -2.0], [0.1, 0.1, -1.0]], Glyph::default()).visible);
}

#[test]
fn metric_glyph_radius_stays_in_world_metres_under_scaled_placement() {
    let transform =
        DMat4::from_translation(DVec3::new(1.05, 0.0, 0.5)) * DMat4::from_scale(DVec3::splat(0.1));
    let projected = view(1.0).project(
        transform,
        [0.0; 3],
        [[0.0; 3]; 2],
        0.0,
        Glyph {
            metres: 0.1,
            pixels: 0.0,
        },
    );
    assert!(projected.visible);
}

#[test]
fn perspective_projection_handles_distance_and_eye_plane_without_nan() {
    let view = View {
        clip_from_world: DMat4::perspective_rh(1.0, 1.0, 0.1, 100.0),
        ..view(1.0)
    };
    let bounds = [[-0.1, -0.1, -0.1], [0.1, 0.1, 0.1]];
    let near = view.project(
        DMat4::from_translation(DVec3::new(0.0, 0.0, -2.0)),
        [0.0; 3],
        bounds,
        0.0,
        Glyph::default(),
    );
    let far = view.project(
        DMat4::from_translation(DVec3::new(0.0, 0.0, -20.0)),
        [0.0; 3],
        bounds,
        0.0,
        Glyph::default(),
    );
    assert!(near.visible && far.visible && near.error() > 9.0 * far.error());
    let eye = view.project(DMat4::IDENTITY, [0.0; 3], bounds, 0.0, Glyph::default());
    assert!(eye.visible && eye.error().is_infinite());
}

#[test]
fn large_source_offsets_use_existing_rebase_and_culled_pages_are_counted() {
    let budget = CpuBudget::new(4 << 20);
    let points = [[1e15, 1e15, 1e15], [1e15 + 0.25, 1e15 + 0.125, 1e15]];
    let (schema, page) = fixture(&points, vec![]);
    let geometry =
        Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
    let mut visible = input(&geometry, 1, 0, 0);
    visible.world_from_local = DMat4::from_translation(DVec3::new(0.0, 0.0, 0.5));
    let mut hidden = input(&geometry, 2, 0, 0);
    hidden.world_from_local = DMat4::from_translation(DVec3::new(10.0, 0.0, 0.5));
    let cut = choose(&budget, &[visible, hidden], 10);
    assert_eq!(ids(&cut).len(), 2);
    assert_eq!(cut.statistics.valid, 4);
    assert_eq!(cut.statistics.culled, 2);
    assert_eq!(cut.pages[1].count, 0);
}

#[test]
fn equal_source_ids_in_distinct_scans_stay_distinct_and_duplicate_pages_fail() {
    let budget = CpuBudget::new(4 << 20);
    let (schema, page) = fixture(&[[0.0, 0.0, 0.5], [0.5, 0.5, 0.5]], vec![]);
    let geometry =
        Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
    let inputs = [input(&geometry, 1, 10, 0), input(&geometry, 1, 11, 0)];
    let cut = choose(&budget, &inputs, 10);
    assert_eq!(ids(&cut).len(), 4);
    assert!(ids(&cut).contains(&(1, 10, u64::MAX)));
    assert!(ids(&cut).contains(&(1, 11, u64::MAX)));
    assert!(
        select(
            &budget,
            &[input(&geometry, 1, 10, 0), input(&geometry, 1, 10, 0)],
            view(1.0),
            limits(10),
            None,
            |_| true,
            || Ok(())
        )
        .is_err()
    );
}

#[test]
fn empty_invalid_unavailable_and_invalid_view_inputs_are_explicit() {
    let budget = CpuBudget::new(4 << 20);
    let (schema, page) = fixture(&[[0.0; 3]; 3], vec![0]);
    let invalid =
        Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
    assert_eq!(
        ids(&choose(&budget, &[input(&invalid, 1, 0, 0)], 0)),
        vec![]
    );
    assert_eq!(ids(&choose(&budget, &[], 0)), vec![]);
    let geometry = geometry(&budget, 5, 0, 0.1);
    let inputs = [input(&geometry, 1, 0, 0)];
    assert!(matches!(
        select(
            &budget,
            &inputs,
            view(1.0),
            limits(10),
            None,
            |_| false,
            || Ok(())
        ),
        Err(Error::UnavailableRoot(_))
    ));
    for bad in [
        View {
            viewport: [0, 100],
            ..view(1.0)
        },
        view(f64::NAN),
        View {
            hysteresis: 1.0,
            ..view(1.0)
        },
    ] {
        assert!(select(&budget, &inputs, bad, limits(10), None, |_| true, || Ok(())).is_err());
    }
}

#[test]
fn cancellation_and_budget_denial_release_every_partial_selection_allocation() {
    let source_budget = CpuBudget::new(8 << 20);
    let geometry = geometry(&source_budget, 513, 0, 0.1);
    let inputs = [input(&geometry, 1, 0, 0)];
    let budget = CpuBudget::new(8 << 20);
    let calls = Cell::new(0);
    drop(
        select(
            &budget,
            &inputs,
            view(0.01),
            limits(1000),
            None,
            |_| true,
            || {
                calls.set(calls.get() + 1);
                Ok(())
            },
        )
        .unwrap(),
    );
    assert_eq!(budget.usage().committed(), 0);
    for stop in 1..=calls.get() {
        let current = Cell::new(0);
        let result = select(
            &budget,
            &inputs,
            view(0.01),
            limits(1000),
            None,
            |_| true,
            || {
                current.set(current.get() + 1);
                if current.get() == stop {
                    Err(StorageError::Conflict)
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(
            result,
            Err(Error::Storage(StorageError::Conflict))
        ));
        assert_eq!(budget.usage().committed(), 0);
    }
    for limit in [0, 1, 128, 512] {
        let budget = CpuBudget::new(limit);
        assert!(matches!(
            select(
                &budget,
                &inputs,
                view(0.01),
                limits(1000),
                None,
                |_| true,
                || Ok(())
            ),
            Err(Error::Storage(StorageError::Budget { .. }))
        ));
        assert_eq!(budget.usage().committed(), 0);
    }
}

#[test]
fn rapid_camera_changes_keep_cuts_bounded_and_valid_without_rebuilding_geometry() {
    let budget = CpuBudget::new(16 << 20);
    let geometry = geometry(&budget, 4097, 0, 0.1);
    let index = geometry.hierarchy.order.values.as_ptr();
    let mut previous = None;
    for frame in 0..50 {
        let mut input = input(&geometry, 1, 0, 0);
        input.world_from_local.w_axis.x += if frame % 3 == 0 { 10.0 } else { 0.0 };
        let cap = if frame % 2 == 0 { 17 } else { 513 };
        let cut = select(
            &budget,
            &[input],
            view(2.0),
            limits(cap),
            previous.as_ref(),
            |_| true,
            || Ok(()),
        )
        .unwrap();
        assert!(ids(&cut).len() <= cap as usize);
        if frame % 3 == 0 {
            assert_eq!(cut.statistics.selected, 0);
        } else {
            assert!(cut.statistics.selected > 0);
        }
        assert_eq!(geometry.hierarchy.order.values.as_ptr(), index);
        previous = Some(cut);
    }
    drop((previous, geometry));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn charged_priority_heap_has_stable_ties() {
    let budget = CpuBudget::new(1 << 20);
    let mut heap = List::new(&budget, 100, MemoryClass::Scratch).unwrap();
    for entry in (0..100).rev() {
        push(
            &mut heap,
            Candidate {
                entry,
                error: (entry % 7) as f64,
            },
        )
        .unwrap();
    }
    let mut expected: Vec<_> = (0..100).collect();
    expected.sort_unstable_by_key(|entry| (std::cmp::Reverse(entry % 7), *entry));
    for entry in expected {
        assert_eq!(pop(&mut heap).unwrap().entry, entry);
    }
    assert!(pop(&mut heap).is_none());
}

#[test]
fn million_point_global_cut_selects_from_all_100k_1m_and_5m_source_datasets() {
    let source_budget = CpuBudget::new(1 << 30);
    let budget = CpuBudget::new(32 << 20);
    let mut sources = Vec::new();
    for (dataset, count) in [100_000usize, 1_000_000, 5_000_000].into_iter().enumerate() {
        for first in (0..count).step_by(65_536) {
            sources.push((
                dataset as u64 + 1,
                (first / 65_536) as u64,
                unique_geometry(&source_budget, (count - first).min(65_536), first as u64),
            ));
        }
    }
    let inputs: Vec<_> = sources
        .iter()
        .map(|(dataset, page, geometry)| input(geometry, *dataset, 0, *page))
        .collect();
    let cut = choose(&budget, &inputs, 1_000_000);
    let selected = ids(&cut);
    assert_eq!(cut.statistics.valid, 6_100_000);
    assert_eq!(cut.statistics.represented, 6_100_000);
    assert!((900_000..=1_000_000).contains(&selected.len()));
    assert_eq!(
        selected.iter().map(|id| id.0).collect::<BTreeSet<_>>(),
        BTreeSet::from([1, 2, 3])
    );
    assert!(budget.usage().peak <= 32 << 20);
    drop(cut);
    drop(inputs);
    drop(sources);
    assert_eq!(budget.usage().committed(), 0);
    assert_eq!(source_budget.usage().committed(), 0);
}
