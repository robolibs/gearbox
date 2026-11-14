use super::*;

#[test]
fn normal_survey_fits_with_its_real_coarse_index_and_shared_style_bounds() {
    let coarse_points = 19_584;
    let pages = 77;
    let point_bytes = 16;
    let source_bytes = 5_000_000 * point_bytes + coarse_points * point_bytes + pages * 1024;
    let reserve = headroom(
        1_000_000,
        [coarse_points].into_iter(),
        1,
        4 << 20,
        14_720_000,
        0,
    );
    assert_eq!(reserve, 27_203_328);
    assert_eq!(source_bytes + reserve, 107_595_520);
    assert!(source_bytes + reserve <= 256 << 20);
    let old_reserve = 8_000_000 + 8 * 1024 * 1024 + 14_720_000 + pages * 16_384;
    assert!(reserve < old_reserve);
}

#[test]
fn index_headroom_caps_each_fallback_but_keeps_upload_and_external_reservations() {
    assert_eq!(
        headroom(100, [0, 50, 500].into_iter(), 2, 1024, 300, 400),
        (100 + 100 + 50 + 100 + 100) * 4 + 2 * 16_384 + 2048 + 700
    );
    assert_eq!(headroom(100, [].into_iter(), 0, 1024, 300, 400), 2748);
    assert_eq!(
        headroom(u32::MAX, [u64::MAX].into_iter(), usize::MAX, u64::MAX, 1, 1),
        u64::MAX
    );
}

fn solve(costs: &[u64], wants: &[(usize, usize, f64)], ceiling: u64) -> (Vec<usize>, u64) {
    let budget = CpuBudget::new(1 << 20);
    let mut pages = List::new(&budget, costs.len(), MemoryClass::Metadata).unwrap();
    for (page, full_bytes) in costs.iter().enumerate() {
        pages
            .push(Page {
                key: lod::PageKey {
                    incarnation: 1,
                    scan: 0,
                    page: page as u64,
                },
                coarse_bytes: 1,
                full_bytes: *full_bytes,
                full: false,
            })
            .unwrap();
    }
    let mut demands = List::new(&budget, wants.len(), MemoryClass::Scratch).unwrap();
    for &(view, page, error) in wants {
        demands.push(Demand { view, page, error }).unwrap();
    }
    let count = wants.iter().map(|(view, _, _)| view + 1).max().unwrap_or(0);
    let result = plan(&budget, pages, demands, count, ceiling).unwrap();
    assert!(result.bytes <= ceiling);
    let selected = result
        .pages
        .iter()
        .enumerate()
        .filter(|(_, page)| page.full)
        .map(|(index, _)| index)
        .collect();
    let bytes = result.bytes;
    drop(result);
    assert_eq!(budget.usage().committed(), 0);
    (selected, bytes)
}

#[test]
fn divergent_views_receive_refinement_before_one_consumes_all_capacity() {
    let demands = [
        (0, 0, 9.0),
        (0, 1, 8.0),
        (0, 2, 7.0),
        (1, 3, 3.0),
        (1, 4, 2.0),
        (1, 5, 1.0),
    ];
    assert_eq!(solve(&[10; 6], &demands, 46), (vec![0, 1, 3, 4], 46));
    let mut permuted = demands;
    permuted.reverse();
    assert_eq!(
        solve(&[10; 6], &permuted, 46),
        solve(&[10; 6], &demands, 46)
    );
}

#[test]
fn shared_pages_are_charged_once_and_credit_both_views() {
    let demands = [(0, 0, 9.0), (0, 1, 5.0), (1, 0, 9.0), (1, 2, 4.0)];
    assert_eq!(solve(&[10; 3], &demands, 33), (vec![0, 1, 2], 33));
    assert_eq!(solve(&[10; 3], &demands, 23), (vec![0, 1], 23));
}

#[test]
fn unused_shares_and_unaffordable_pages_do_not_block_remaining_work() {
    let demands = [(0, 0, 9.0), (0, 1, 8.0), (1, 2, 5.0), (1, 3, 4.0)];
    assert_eq!(solve(&[100, 4, 4, 4], &demands, 16), (vec![1, 2, 3], 16));
    assert_eq!(
        solve(&[4, 4, 4], &[(0, 0, 1.0), (1, 1, 2.0), (1, 2, 1.0)], 15),
        (vec![0, 1, 2], 15)
    );
}

#[test]
fn fairness_accounts_bytes_instead_of_page_counts() {
    let demands = [
        (0, 0, 9.0),
        (0, 1, 8.0),
        (1, 2, 4.0),
        (1, 3, 3.0),
        (1, 4, 2.0),
        (1, 5, 1.0),
    ];
    assert_eq!(
        solve(&[20, 20, 5, 5, 5, 5], &demands, 46),
        (vec![0, 2, 3, 4, 5], 46)
    );
}

#[test]
fn empty_and_coarse_only_plans_respect_limits() {
    assert_eq!(solve(&[], &[], 0), (vec![], 0));
    assert_eq!(
        solve(&[10; 2], &[(0, 0, f64::INFINITY), (1, 1, 1.0)], 2),
        (vec![], 2)
    );
}

#[test]
fn invalid_demand_and_cpu_denial_release_all_ownership() {
    let budget = CpuBudget::new(1 << 20);
    for wants in [vec![(0, 0, 1.0)], vec![(0, 0, f64::NAN)]] {
        let pages = List::default();
        let mut demands = List::new(&budget, wants.len(), MemoryClass::Scratch).unwrap();
        for (view, page, error) in wants {
            demands.push(Demand { view, page, error }).unwrap();
        }
        assert!(plan(&budget, pages, demands, 1, 10).is_err());
        assert_eq!(budget.usage().committed(), 0);
    }
    let pressure = budget.reserve(budget.usage().limit).unwrap();
    assert!(plan(&budget, List::default(), List::default(), 1, 0).is_err());
    drop(pressure);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn small_near_clouds_are_not_starved_by_one_large_distant_page() {
    let large = 3 << 20;
    let small = 5 * 1024;
    let mut costs = vec![large];
    let mut wants = vec![(0, 0, 160.0)];
    for page in 1..=10 {
        costs.push(small);
        wants.push((0, page, 90.0));
    }
    let (selected, bytes) = solve(&costs, &wants, large + 4 * small);
    assert_eq!(selected, (1..=10).collect::<Vec<_>>());
    assert_eq!(bytes, 10 * small as u64 + 11);
    let (selected, _) = solve(&costs, &wants, large + 10 * small + 11);
    assert_eq!(selected, (0..=10).collect::<Vec<_>>());
}

#[test]
fn equal_page_costs_keep_projected_error_order() {
    let costs = [1000; 4];
    let wants = [(0, 0, 1.0), (0, 1, 4.0), (0, 2, 3.0), (0, 3, 2.0)];
    let (selected, _) = solve(&costs, &wants, 2000 + 4);
    assert_eq!(selected, vec![1, 2]);
}
