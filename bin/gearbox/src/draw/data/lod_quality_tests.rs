use bevy::math::DVec3;

use super::*;
use crate::draw::data::tests::{fixture, stored};

fn survey_point(index: usize, side: usize) -> [f64; 3] {
    let span = (side - 1) as f32;
    let x = (index % side) as f32 / span * 30.0 - 15.0;
    let y = (index / side) as f32 / span * 30.0 - 15.0;
    let hill =
        |cx: f32, cy: f32, scale: f32| (-((x - cx).powi(2) + (y - cy).powi(2)) / scale).exp();
    let z = 1.4
        + 6.0 * hill(-6.0, 4.0, 22.0)
        + 4.0 * hill(6.0, -4.0, 18.0)
        + 0.45 * (x * 0.6).sin() * (y * 0.5).cos();
    [x, y, z].map(f64::from)
}

fn survey(budget: &CpuBudget, count: usize) -> Vec<Arc<Geometry>> {
    let side = (count as f64).sqrt().ceil() as usize;
    (0..count)
        .step_by(65_536)
        .map(|first| {
            let end = (first + 65_536).min(count);
            let points: Vec<_> = (first..end).map(|i| survey_point(i, side)).collect();
            let (schema, mut page) = fixture(&points, vec![]);
            page.columns[1].values = (first..end)
                .flat_map(|i| (u64::MAX - i as u64).to_le_bytes())
                .collect();
            Geometry::prepare(budget, &schema, stored(budget, &schema, &page, [0.0; 3])).unwrap()
        })
        .collect()
}

fn overview() -> View {
    let bearing = 170.0f64.to_radians();
    let pitch = 48.0f64.to_radians();
    let target = DVec3::new(0.0, 0.0, 3.0);
    let eye = target
        + 58.0
            * DVec3::new(
                bearing.sin() * pitch.cos(),
                bearing.cos() * pitch.cos(),
                pitch.sin(),
            );
    View {
        clip_from_world: DMat4::perspective_infinite_reverse_rh(
            std::f64::consts::FRAC_PI_4,
            1.84,
            0.1,
        ) * DMat4::look_at_rh(eye, target, DVec3::Z),
        viewport: [1840, 1000],
        error_px: 1.5,
        hysteresis: 0.15,
    }
}

fn quantiles(mut values: Vec<f64>) -> [f64; 4] {
    values.sort_unstable_by(f64::total_cmp);
    [
        values[0],
        values[values.len() / 2],
        values[values.len() * 95 / 100],
        *values.last().unwrap(),
    ]
}

#[test]
fn paged_curved_survey_preserves_spatial_coverage_with_one_million_selected_points() {
    let source_budget = CpuBudget::new(1 << 30);
    let selection_budget = CpuBudget::new(32 << 20);
    let sources = survey(&source_budget, 5_000_000);
    let inputs: Vec<_> = sources
        .iter()
        .enumerate()
        .map(|(page, geometry)| Input {
            key: PageKey {
                incarnation: 1,
                scan: 0,
                page: page as u64,
            },
            geometry: Arc::downgrade(geometry),
            metadata: &geometry.metadata,
            world_from_local: DMat4::from_translation(DVec3::from_array(geometry.rebase_m)),
            glyph: Glyph {
                metres: 0.0,
                pixels: 0.8,
            },
        })
        .collect();
    let view = overview();
    let cut = select(
        &selection_budget,
        &inputs,
        view,
        Limits {
            points: 1_000_000,
            bytes: 4_000_000,
            bytes_per_point: 4,
        },
        None,
        |_| true,
        || Ok(()),
    )
    .unwrap();
    let diameters: Vec<_> = sources
        .iter()
        .flat_map(|source| {
            source
                .hierarchy
                .nodes
                .values
                .iter()
                .filter(|node| node.children.is_none())
                .map(|node| node.error_m)
        })
        .collect();
    let mut bins = vec![0u32; 128 * 128];
    for (source, page) in sources.iter().zip(&cut.pages) {
        for ordinal in &cut.indices[page.first as usize..(page.first + page.count) as usize] {
            let position = source.source_position(*ordinal as usize).unwrap();
            let [x, y] = [position[0], position[1]]
                .map(|v| (((v + 15.0) / 30.0 * 128.0).floor() as usize).min(127));
            bins[y * 128 + x] += 1;
        }
    }
    let occupied = bins.iter().filter(|count| **count != 0).count();
    let mut populations = [0usize; 10];
    for piece in &cut.pieces {
        populations[(piece.selected.ilog2() as usize).min(9)] += 1;
    }
    eprintln!(
        "{}",
        serde_json::json!({
            "source_count": cut.statistics.valid, "selected": cut.statistics.selected,
            "represented": cut.statistics.represented, "max_error_px": cut.statistics.max_error_px,
            "budget_limited": cut.statistics.budget_limited_branches,
            "leaf_diameter_m_quantiles": quantiles(diameters), "piece_selected_log2": populations,
            "occupied_ground_bins": occupied, "ground_bins": bins.len(),
            "selection_peak_bytes": selection_budget.usage().peak,
        })
    );
    assert_eq!(cut.statistics.valid, 5_000_000);
    assert_eq!(cut.statistics.represented, 5_000_000);
    assert!(cut.statistics.selected <= 1_000_000);
    assert!(selection_budget.usage().peak <= 32 << 20);
    assert!(
        occupied >= bins.len() * 99 / 100,
        "selected only {occupied}/{} populated ground bins",
        bins.len()
    );
    assert!(cut.statistics.max_error_px <= view.error_px * (1.0 + view.hysteresis));
    drop((cut, inputs));
    drop(sources);
    assert_eq!(selection_budget.usage().committed(), 0);
    assert_eq!(source_budget.usage().committed(), 0);
}
