use super::*;

#[test]
fn view_selection_uploads_share_the_quantum_without_key_collisions() {
    let budget = budget();
    let mut scheduler = Scheduler::default();
    let keys = [
        Key::Typed(1),
        Key::Legacy(1),
        Key::Selection(1, 1),
        Key::Selection(1, 2),
    ];
    let mut totals = [0; 4];
    for _ in 0..8 {
        let mut jobs: Vec<_> = keys.iter().map(|key| Job::new(*key, 0)).collect();
        let mut remaining = QUANTUM;
        scheduler
            .run(&budget, &mut jobs, &mut remaining, |key, _, credit| {
                let index = keys.iter().position(|candidate| *candidate == key).unwrap();
                totals[index] += *credit;
                *credit = 0;
                Err(Issue::Deferred)
            })
            .unwrap();
        assert_eq!(remaining, 0);
    }
    assert_eq!(totals, [2 * QUANTUM; 4]);
    drop(scheduler);
    assert_eq!(budget.usage().committed(), 0);
}

fn budget() -> CpuBudget {
    CpuBudget::new(1 << 20)
}

#[test]
fn recreated_typed_groups_cannot_overtake_a_waiting_legacy_group() {
    for new_groups in [64, 128] {
        let budget = budget();
        let mut scheduler = Scheduler::default();
        let mut legacy_bytes = 0;
        for frame in 0..8 {
            let mut jobs: Vec<_> = (0..new_groups)
                .map(|index| Job::new(Key::Typed(1 + frame * new_groups + index), index))
                .collect();
            jobs.push(Job::new(Key::Legacy(1), 999));
            scheduler.prune(&budget, jobs.iter().map(|job| job.key));
            let mut remaining = QUANTUM * 64;
            scheduler
                .run(&budget, &mut jobs, &mut remaining, |key, _, credit| {
                    if key == Key::Legacy(1) {
                        legacy_bytes += *credit;
                    }
                    *credit = 0;
                    Err(Issue::Deferred)
                })
                .unwrap();
            assert_eq!(remaining, 0);
            if frame > 0 {
                assert_eq!(
                    legacy_bytes,
                    frame * QUANTUM,
                    "new incarnation groups overtook the surviving legacy group"
                );
            }
        }
        scheduler.prune(&budget, []);
        assert_eq!(budget.usage().allocated, 0);
    }
}

#[test]
fn many_pages_get_one_group_share_and_legacy_cannot_starve() {
    let budget = budget();
    let mut scheduler = Scheduler::default();
    let mut uploaded = [0; 3];
    for generation in 0..9 {
        let mut jobs: Vec<_> = (0..20)
            .map(|page| Job::new(Key::Typed(1), generation * 100 + page))
            .collect();
        jobs.push(Job::new(Key::Typed(2), 80));
        jobs.push(Job::new(Key::Legacy(3), 90));
        scheduler.prune(&budget, jobs.iter().map(|job| job.key));
        let mut remaining = QUANTUM;
        scheduler
            .run(&budget, &mut jobs, &mut remaining, |key, _, credit| {
                let index = match key {
                    Key::Typed(1) => 0,
                    Key::Typed(2) => 1,
                    Key::Legacy(3) => 2,
                    _ => unreachable!(),
                };
                uploaded[index] += *credit;
                *credit = 0;
                Err(Issue::Deferred)
            })
            .unwrap();
        assert_eq!(remaining, 0);
        if generation % 3 == 2 {
            assert!(uploaded.iter().all(|bytes| *bytes == uploaded[0]));
        }
    }
    assert_eq!(uploaded, [QUANTUM * 3; 3]);
}

#[test]
fn unused_credit_is_reused_in_the_same_frame_and_no_progress_terminates() {
    let budget = budget();
    let mut scheduler = Scheduler::default();
    let mut jobs = [
        Job::new(Key::Typed(1), 1),
        Job::new(Key::Typed(2), 2),
        Job::new(Key::Legacy(3), 3),
    ];
    let mut remaining = QUANTUM * 3;
    let mut calls = 0;
    scheduler
        .run(&budget, &mut jobs, &mut remaining, |key, _, credit| {
            calls += 1;
            match key {
                Key::Typed(1) => Ok(()),
                Key::Typed(2) => Err(Issue::GpuAllocation),
                _ => {
                    *credit = 0;
                    Err(Issue::Deferred)
                }
            }
        })
        .unwrap();
    assert_eq!(remaining, 0);
    assert!(calls < 10);
    assert_eq!(jobs[0].result, Ok(()));
    assert_eq!(jobs[1].result, Err(Issue::GpuAllocation));
    let mut remaining = QUANTUM * 8;
    let mut jobs = [Job::new(Key::Typed(1), 1), Job::new(Key::Legacy(3), 3)];
    let mut calls = 0;
    scheduler
        .run(&budget, &mut jobs, &mut remaining, |_, _, _| {
            calls += 1;
            Err(Issue::Deferred)
        })
        .unwrap();
    assert_eq!(remaining, QUANTUM * 8);
    assert_eq!(calls, 4);
}

#[test]
fn tiny_tail_does_not_steal_next_frame_priority_from_atomic_uniform() {
    let budget = budget();
    let mut scheduler = Scheduler::default();
    let mut uniforms = 0;
    for _ in 0..6 {
        let mut jobs = [Job::new(Key::Typed(1), 1), Job::new(Key::Legacy(2), 2)];
        let mut remaining = QUANTUM + 4;
        scheduler
            .run(&budget, &mut jobs, &mut remaining, |key, _, credit| {
                if key == Key::Legacy(2) {
                    if *credit < 512 {
                        return Err(Issue::Deferred);
                    }
                    *credit -= 512;
                    uniforms += 1;
                    Ok(())
                } else {
                    *credit = 0;
                    Err(Issue::Deferred)
                }
            })
            .unwrap();
        assert_eq!(remaining, 0);
    }
    assert_eq!(uniforms, 5);
}

#[test]
fn item_cursor_resumes_and_removal_releases_state_under_full_cpu_pressure() {
    let budget = budget();
    let mut scheduler = Scheduler::default();
    let mut seen = Vec::new();
    for _ in 0..4 {
        let mut jobs = [Job::new(Key::Typed(1), 1), Job::new(Key::Typed(1), 2)];
        let mut remaining = QUANTUM;
        scheduler
            .run(&budget, &mut jobs, &mut remaining, |_, item, credit| {
                if *credit == 0 {
                    return Err(Issue::Deferred);
                }
                seen.push(item);
                *credit = 0;
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(seen, [1, 2, 1, 2]);
    assert!(budget.usage().allocated > 0);
    let pressure = budget
        .reserve(budget.usage().limit - budget.usage().committed())
        .unwrap();
    scheduler.prune(&budget, []);
    assert_eq!(budget.usage().allocated, 0);
    assert!(scheduler.next.is_none());
    drop(pressure);
}

#[test]
fn metadata_denial_precedes_dispatch_and_can_retry_without_replacing_jobs() {
    let budget = budget();
    let mut scheduler = Scheduler::default();
    let mut jobs = [Job::new(Key::Typed(1), 1)];
    let pressure = budget.reserve(budget.usage().limit).unwrap();
    let mut remaining = QUANTUM;
    assert!(
        scheduler
            .run(&budget, &mut jobs, &mut remaining, |_, _, _| panic!(
                "not admitted"
            ))
            .is_err()
    );
    assert_eq!(remaining, QUANTUM);
    drop(pressure);
    scheduler
        .run(&budget, &mut jobs, &mut remaining, |_, _, credit| {
            *credit -= 4;
            Ok(())
        })
        .unwrap();
    assert_eq!(remaining, QUANTUM - 4);
    assert_eq!(jobs[0].result, Ok(()));
}

#[test]
fn zero_credit_still_checks_resident_items_without_reordering_productive_turns() {
    let budget = budget();
    let mut scheduler = Scheduler::default();
    let mut jobs = [Job::new(Key::Typed(1), 1), Job::new(Key::Legacy(2), 2)];
    let mut remaining = QUANTUM;
    scheduler
        .run(&budget, &mut jobs, &mut remaining, |key, _, credit| {
            if key == Key::Typed(1) {
                *credit = 0;
            }
            Ok(())
        })
        .unwrap();
    assert!(jobs.iter().all(|job| job.result.is_ok()));
    assert_eq!(scheduler.next, Some(Key::Legacy(2)));
}

#[test]
fn removed_successor_resolves_to_an_existing_group_before_admitting_new_arrivals() {
    let budget = budget();
    for turns in [1, 2] {
        let mut scheduler = Scheduler::default();
        let mut jobs = [
            Job::new(Key::Typed(1), 1),
            Job::new(Key::Typed(2), 2),
            Job::new(Key::Legacy(3), 3),
        ];
        let mut remaining = QUANTUM * turns;
        scheduler
            .run(&budget, &mut jobs, &mut remaining, |_, _, credit| {
                *credit = 0;
                Err(Issue::Deferred)
            })
            .unwrap();
        let survivors = if turns == 1 {
            [Key::Typed(1), Key::Legacy(3)]
        } else {
            [Key::Typed(1), Key::Typed(2)]
        };
        let expected = if turns == 1 {
            Key::Legacy(3)
        } else {
            Key::Typed(1)
        };
        let pressure = budget
            .reserve(budget.usage().limit - budget.usage().committed())
            .unwrap();
        scheduler.prune(&budget, survivors);
        assert_eq!(scheduler.next, Some(expected));
        drop(pressure);
        let mut jobs: Vec<_> = survivors
            .into_iter()
            .enumerate()
            .map(|(item, key)| Job::new(key, item as u64))
            .collect();
        jobs.push(Job::new(Key::Typed(4), 4));
        let mut remaining = QUANTUM;
        let mut first = None;
        scheduler
            .run(&budget, &mut jobs, &mut remaining, |key, _, credit| {
                if *credit > 0 {
                    first.get_or_insert(key);
                }
                *credit = 0;
                Err(Issue::Deferred)
            })
            .unwrap();
        assert_eq!(first, Some(expected));
        scheduler.prune(&budget, []);
        assert_eq!(budget.usage().allocated, 0);
    }
}

#[test]
fn staging_bound_covers_repeated_partial_writes_and_checked_arithmetic() {
    assert_eq!(staging_slots(1, 1, QUANTUM * 5 + 4).unwrap(), 13);
    assert_eq!(staging_slots(0, 0, u64::MAX).unwrap(), 0);
    assert!(staging_slots(usize::MAX, 1, QUANTUM).is_err());
    assert!(staging_slots(1, usize::MAX, QUANTUM).is_err());
}
