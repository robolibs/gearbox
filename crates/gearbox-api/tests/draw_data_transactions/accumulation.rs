use super::*;

fn accumulated(clock: AgeClock, scans: u32, age: f64, bytes: u64) -> DataDescriptor {
    DataDescriptor {
        stream: StreamMode::Accumulate {
            max_scans: scans,
            max_age_s: age,
            max_bytes: bytes,
            age_clock: clock,
        },
        ..descriptor()
    }
}

fn captured(key: u64, time: f64, source_ns: u64, count: u64) -> ScanDescriptor {
    let mut scan = scan(key, count);
    let capture = scan.frame.capture.as_mut().unwrap();
    capture.simulation_time_s = time;
    capture.source_time_ns = Some(source_ns);
    scan.frame.capture_transform.as_mut().unwrap().stamp = capture.clone();
    scan
}

fn stage(
    store: &mut StreamingStore,
    lease: u64,
    id: u64,
    descriptor: &DataDescriptor,
    base: Option<DatasetRevision>,
    mode: CommitMode,
    scans: &[(ScanDescriptor, Vec<u64>)],
) -> DataRequest {
    let pages: Vec<_> = scans
        .iter()
        .map(|(scan, ids)| page(scan.scan_key, 1, ids))
        .collect();
    let request = begin(
        lease,
        id,
        descriptor.clone(),
        mode,
        base,
        scans.len() as u32,
        pages.len() as u32,
        0,
        0,
        pages.iter().map(|(page, _)| page.byte_len).sum(),
    );
    ok(store, &request, 0);
    for ((scan, _), (page, bytes)) in scans.iter().zip(&pages) {
        declare_scan(store, &request, scan, 0);
        upload(store, &request, page, bytes, 0);
    }
    request
}

fn keys(generation: &Generation) -> Vec<u64> {
    generation
        .scans()
        .iter()
        .map(|scan| scan.value().scan_key)
        .collect()
}

#[test]
fn count_eviction_accepts_a_bounded_union_at_the_host_limit_and_shares_survivors() {
    let mut store = StreamingStore::new(
        EPOCH,
        DataLimits {
            scans: 2,
            pages: 2,
            ..limits()
        },
    )
    .unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Source, 2, 100.0, 4096);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[
            (captured(1, 1.0, 1, 1), vec![1]),
            (captured(2, 2.0, 2, 1), vec![2]),
        ],
    );
    let base = commit(&mut store, &first, 0);
    let old = retained(&store, &descriptor.key);
    let third = captured(3, 3.0, 3, 1);
    let next = stage(
        &mut store,
        lease,
        2,
        &descriptor,
        Some(base),
        CommitMode::Delta,
        &[(third.clone(), vec![3])],
    );
    let changed = commit(&mut store, &next, 0);
    let new = retained(&store, &descriptor.key);
    assert_eq!(keys(&new), [2, 3]);
    assert!(Arc::ptr_eq(&new.pages()[0], &old.pages()[1]));
    assert_eq!(new.scans()[1].value(), &third);
    assert_eq!(store.counters().evicted_scans, 1);
    assert_eq!(store.counters().dropped_scans, 0);
    assert_eq!(commit(&mut store, &next, 0), changed);
    assert_eq!(store.counters().evicted_scans, 1);
    assert_eq!(
        store.usage().memory.page_bytes,
        new.counts().payload_bytes + old.pages()[0].descriptor().byte_len
    );
    drop(old);
    assert_eq!(store.usage().memory.page_bytes, new.counts().payload_bytes);
}

#[test]
fn byte_eviction_removes_whole_oldest_scans_not_individual_samples() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let bytes = page(1, 1, &[1, 2]).0.byte_len;
    let descriptor = accumulated(AgeClock::Simulation, 8, 100.0, bytes);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[(captured(1, 1.0, 1, 2), vec![1, 2])],
    );
    let base = commit(&mut store, &first, 0);
    let next = stage(
        &mut store,
        lease,
        2,
        &descriptor,
        Some(base),
        CommitMode::Delta,
        &[(captured(2, 2.0, 2, 1), vec![9])],
    );
    commit(&mut store, &next, 0);
    let new = retained(&store, &descriptor.key);
    assert_eq!(keys(&new), [2]);
    assert_eq!(new.counts().source_samples, 1);
    assert_eq!(
        new.pages()[0].source_ids().unwrap().collect::<Vec<_>>(),
        [9]
    );
    assert_eq!(store.counters().evicted_scans, 1);
}

#[test]
fn oversized_single_scan_rejects_the_submission_without_evicting_the_old_map() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Source, 8, 100.0, page(1, 1, &[1]).0.byte_len);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[(captured(1, 1.0, 1, 1), vec![1])],
    );
    let base = commit(&mut store, &first, 0);
    let old = retained(&store, &descriptor.key);
    let next = stage(
        &mut store,
        lease,
        2,
        &descriptor,
        Some(base),
        CommitMode::Delta,
        &[(captured(2, 2.0, 2, 2), vec![2, 3])],
    );
    assert_eq!(
        call(&mut store, &edit(&next, data_op::COMMIT), 0).code,
        data_code::BUDGET
    );
    assert!(Arc::ptr_eq(&old, &retained(&store, &descriptor.key)));
    assert_eq!(store.counters().evicted_scans, 0);
    assert_eq!(store.usage().memory.page_bytes, old.counts().payload_bytes);
}

#[test]
fn source_age_uses_integer_timestamp_differences_at_u64_max() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Source, 8, 2e-9, 4096);
    let scans = [
        (captured(1, 100.0, u64::MAX - 2, 1), vec![1]),
        (captured(2, 0.0, u64::MAX - 1, 1), vec![2]),
        (captured(3, 1.0, u64::MAX, 1), vec![3]),
    ];
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &scans,
    );
    commit(&mut store, &first, 0);
    assert_eq!(keys(&retained(&store, &descriptor.key)), [2, 3]);
    assert_eq!(store.counters().dropped_scans, 1);
    assert_eq!(store.counters().evicted_scans, 0);
    store.advance_simulation(1e8).unwrap();
    assert_eq!(keys(&retained(&store, &descriptor.key)), [2, 3]);
}

#[test]
fn late_source_scans_do_not_rewind_the_watermark_after_empty_replacement() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Source, 4, 1.0, 4096);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[(captured(1, 0.0, 10_000_000_000, 1), vec![1])],
    );
    let base = commit(&mut store, &first, 0);
    let empty = stage(
        &mut store,
        lease,
        2,
        &descriptor,
        Some(base),
        CommitMode::Replace,
        &[],
    );
    let base = commit(&mut store, &empty, 0);
    let late = stage(
        &mut store,
        lease,
        3,
        &descriptor,
        Some(base),
        CommitMode::Delta,
        &[(captured(2, 0.0, 9_000_000_000, 1), vec![2])],
    );
    let base = commit(&mut store, &late, 0);
    assert!(keys(&retained(&store, &descriptor.key)).is_empty());
    assert_eq!(retained(&store, &descriptor.key).scan_high_watermark(), 2);
    assert_eq!(store.counters().dropped_scans, 1);
    let current = stage(
        &mut store,
        lease,
        4,
        &descriptor,
        Some(base),
        CommitMode::Delta,
        &[(captured(3, 0.0, 10_000_000_001, 1), vec![3])],
    );
    commit(&mut store, &current, 0);
    assert_eq!(keys(&retained(&store, &descriptor.key)), [3]);
}

#[test]
fn changed_clock_or_epoch_is_rejected_even_after_all_scans_are_removed() {
    for epoch in [false, true] {
        let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
        let lease = open(&mut store, 0);
        let descriptor = accumulated(AgeClock::Source, 4, 10.0, 4096);
        let first = stage(
            &mut store,
            lease,
            1,
            &descriptor,
            None,
            CommitMode::Replace,
            &[(captured(1, 0.0, 10, 1), vec![1])],
        );
        let base = commit(&mut store, &first, 0);
        let empty = stage(
            &mut store,
            lease,
            2,
            &descriptor,
            Some(base),
            CommitMode::Replace,
            &[],
        );
        let base = commit(&mut store, &empty, 0);
        let mut changed = captured(2, 0.0, 11, 1);
        let capture = changed.frame.capture.as_mut().unwrap();
        if epoch {
            capture.scene_epoch = 2;
        } else {
            capture.source_clock = Some("other-clock".into());
        }
        changed.frame.capture_transform.as_mut().unwrap().stamp = capture.clone();
        let next = stage(
            &mut store,
            lease,
            3,
            &descriptor,
            Some(base),
            CommitMode::Delta,
            &[(changed, vec![2])],
        );
        assert_eq!(
            call(&mut store, &edit(&next, data_op::COMMIT), 0).code,
            data_code::INVALID
        );
        assert!(keys(&retained(&store, &descriptor.key)).is_empty());
    }
}

#[test]
fn simulation_age_advances_only_with_simulation_and_does_not_refresh_lifetime() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    store.advance_simulation(10.0).unwrap();
    let mut descriptor = accumulated(AgeClock::Simulation, 4, 2.0, 4096);
    descriptor.lifetime_s = Some(5.0);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[
            (captured(1, 10.0, 1, 1), vec![1]),
            (captured(2, 11.0, 2, 1), vec![2]),
        ],
    );
    commit(&mut store, &first, 0);
    let old = retained(&store, &descriptor.key);
    store.maintain(500).unwrap();
    store.advance_simulation(10.0).unwrap();
    assert_eq!(keys(&retained(&store, &descriptor.key)), [1, 2]);
    store.advance_simulation(11.999).unwrap();
    assert_eq!(keys(&retained(&store, &descriptor.key)), [1, 2]);
    store.advance_simulation(12.0).unwrap();
    let new = retained(&store, &descriptor.key);
    assert_eq!(keys(&new), [2]);
    assert!(Arc::ptr_eq(&old.pages()[1], &new.pages()[0]));
    assert_eq!(new.retained_at_s(), 10.0);
    assert_eq!(store.counters().evicted_scans, 1);
    store.advance_simulation(13.0).unwrap();
    assert!(keys(&retained(&store, &descriptor.key)).is_empty());
    assert_eq!(store.counters().evicted_scans, 2);
    assert_eq!(store.advance_simulation(15.0).unwrap(), 1);
    store.maintain(501).unwrap();
    assert_eq!(store.registry().lock().unwrap().dataset_len(), 0);
}

#[test]
fn age_publication_invalidates_pending_exact_base_deltas_without_resurrection() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Simulation, 4, 2.0, 4096);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[(captured(1, 0.0, 1, 1), vec![1])],
    );
    let base = commit(&mut store, &first, 0);
    let pending = stage(
        &mut store,
        lease,
        2,
        &descriptor,
        Some(base),
        CommitMode::Delta,
        &[(captured(2, 3.0, 2, 1), vec![2])],
    );
    store.advance_simulation(2.0).unwrap();
    assert_eq!(
        call(&mut store, &edit(&pending, data_op::COMMIT), 0).code,
        data_code::CONFLICT
    );
    assert_eq!(store.counters().evicted_scans, 1);
    assert!(keys(&retained(&store, &descriptor.key)).is_empty());
    assert_eq!(store.usage().memory.reserved, 0);
}

#[test]
fn failed_age_allocation_is_observable_and_retries_after_capacity_returns() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Simulation, 4, 1.0, 4096);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[(captured(1, 0.0, 1, 1), vec![1])],
    );
    let base = commit(&mut store, &first, 0);
    let old = retained(&store, &descriptor.key);
    let usage = store.budget().usage();
    let pressure = store
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    assert_eq!(
        store.advance_simulation(1.0).unwrap_err().code,
        data_code::BUDGET
    );
    assert!(Arc::ptr_eq(&old, &retained(&store, &descriptor.key)));
    assert_eq!(store.counters().evicted_scans, 0);
    let status =
        DatasetStatus::from_json(&ok(&mut store, &status_request(&descriptor.key), 0).metadata)
            .unwrap();
    assert_eq!(status.retained, Some(base));
    assert_eq!(status.failure.unwrap().phase, DataPhase::Retaining);
    store.advance_simulation(1.0).unwrap();
    drop(pressure);
    store.advance_simulation(1.0).unwrap();
    let status =
        DatasetStatus::from_json(&ok(&mut store, &status_request(&descriptor.key), 0).metadata)
            .unwrap();
    assert!(status.failure.is_none());
    assert_eq!(status.retained.unwrap().generation, base.generation + 1);
    assert!(keys(&retained(&store, &descriptor.key)).is_empty());
    assert_eq!(store.counters().evicted_scans, 1);
}

#[test]
fn eviction_never_hides_invalid_cross_page_source_ids() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Simulation, 1, 1.0, 4096);
    store.advance_simulation(100.0).unwrap();
    let (a, ab) = page(1, 1, &[7]);
    let (b, bb) = page(1, 2, &[7]);
    let first = begin(
        lease,
        1,
        descriptor.clone(),
        CommitMode::Replace,
        None,
        2,
        2,
        0,
        0,
        a.byte_len + b.byte_len,
    );
    ok(&mut store, &first, 0);
    declare_scan(&mut store, &first, &captured(1, 0.0, 1, 2), 0);
    declare_scan(&mut store, &first, &captured(2, 100.0, 2, 0), 0);
    upload(&mut store, &first, &a, &ab, 0);
    upload(&mut store, &first, &b, &bb, 0);
    assert_eq!(
        call(&mut store, &edit(&first, data_op::COMMIT), 0).code,
        data_code::INVALID
    );
    assert_eq!(store.registry().lock().unwrap().dataset_len(), 0);
    assert_eq!(store.usage().memory.page_bytes, 0);
}

#[test]
fn whole_scan_eviction_removes_every_page_and_preserves_nullable_counts() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Source, 1, 100.0, 4096);
    let (a, ab) = page(1, 1, &[1]);
    let (b, bb) = page(1, 2, &[2]);
    let first = begin(
        lease,
        1,
        descriptor.clone(),
        CommitMode::Replace,
        None,
        1,
        2,
        0,
        0,
        a.byte_len + b.byte_len,
    );
    ok(&mut store, &first, 0);
    declare_scan(&mut store, &first, &captured(1, 0.0, 1, 2), 0);
    upload(&mut store, &first, &a, &ab, 0);
    upload(&mut store, &first, &b, &bb, 0);
    let base = commit(&mut store, &first, 0);
    let mut cloud = CloudPage::decode(&schema(), &ab).unwrap();
    cloud.columns[0].validity = vec![0];
    let bytes = cloud.encode(&schema()).unwrap();
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 2,
            page_id: 1,
        },
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        ..a
    };
    let next = begin(
        lease,
        2,
        descriptor.clone(),
        CommitMode::Delta,
        Some(base),
        1,
        1,
        0,
        0,
        page.byte_len,
    );
    ok(&mut store, &next, 0);
    declare_scan(&mut store, &next, &captured(2, 1.0, 2, 1), 0);
    upload(&mut store, &next, &page, &bytes, 0);
    commit(&mut store, &next, 0);
    let new = retained(&store, &descriptor.key);
    assert_eq!(keys(&new), [2]);
    assert_eq!(new.counts().pages, 1);
    assert_eq!(new.counts().source_samples, 1);
    assert_eq!(new.counts().valid_samples, 0);
    assert_eq!(store.usage().memory.page_bytes, page.byte_len);
    assert_eq!(store.counters().evicted_scans, 1);
}

#[test]
fn capture_time_orders_eviction_and_equal_times_use_scan_keys() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Source, 2, 100.0, 4096);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[
            (captured(1, 0.0, 3, 0), vec![]),
            (captured(2, 0.0, 1, 0), vec![]),
            (captured(3, 0.0, 2, 0), vec![]),
        ],
    );
    let base = commit(&mut store, &first, 0);
    assert_eq!(keys(&retained(&store, &descriptor.key)), [1, 3]);
    let next = stage(
        &mut store,
        lease,
        2,
        &descriptor,
        Some(base),
        CommitMode::Delta,
        &[(captured(4, 0.0, 3, 0), vec![])],
    );
    let base = commit(&mut store, &next, 0);
    assert_eq!(keys(&retained(&store, &descriptor.key)), [1, 4]);
    let next = stage(
        &mut store,
        lease,
        3,
        &descriptor,
        Some(base),
        CommitMode::Delta,
        &[(captured(5, 0.0, 3, 0), vec![])],
    );
    commit(&mut store, &next, 0);
    assert_eq!(keys(&retained(&store, &descriptor.key)), [4, 5]);
    assert_eq!(store.counters().dropped_scans, 1);
    assert_eq!(store.counters().evicted_scans, 2);
}

#[test]
fn presentation_preserves_watermark_and_explicit_clock_policy_replacement_resets_it() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let mut descriptor = accumulated(AgeClock::Source, 4, 1.0, 4096);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[(captured(1, 0.0, 10_000_000_000, 1), vec![1])],
    );
    let base = commit(&mut store, &first, 0);
    let style = DataRequest {
        lease,
        incarnation: base.incarnation,
        base_generation: base.generation,
        ..req(data_op::PATCH_PRESENTATION)
    }
    .with_key(&descriptor.key)
    .unwrap()
    .with_metadata(&descriptor.presentation)
    .unwrap();
    let styled = ok(&mut store, &style, 0);
    let base = DatasetRevision {
        incarnation: styled.incarnation,
        generation: styled.stored_generation,
    };
    let empty = stage(
        &mut store,
        lease,
        2,
        &descriptor,
        Some(base),
        CommitMode::Replace,
        &[],
    );
    let base = commit(&mut store, &empty, 0);
    let late = stage(
        &mut store,
        lease,
        3,
        &descriptor,
        Some(base),
        CommitMode::Replace,
        &[(captured(2, 0.0, 1, 1), vec![2])],
    );
    let base = commit(&mut store, &late, 0);
    assert!(keys(&retained(&store, &descriptor.key)).is_empty());
    descriptor.stream = StreamMode::Accumulate {
        max_scans: 4,
        max_age_s: 1.0,
        max_bytes: 4096,
        age_clock: AgeClock::Simulation,
    };
    let switch = stage(
        &mut store,
        lease,
        4,
        &descriptor,
        Some(base),
        CommitMode::Replace,
        &[(captured(3, 0.0, 0, 1), vec![3])],
    );
    commit(&mut store, &switch, 0);
    assert_eq!(keys(&retained(&store, &descriptor.key)), [3]);
    assert_eq!(retained(&store, &descriptor.key).scan_high_watermark(), 3);
}

#[test]
fn generated_submission_sequence_matches_a_whole_scan_reference_model() {
    let mut store = StreamingStore::new(
        EPOCH,
        DataLimits {
            scans: 4,
            pages: 4,
            ..limits()
        },
    )
    .unwrap();
    let lease = open(&mut store, 0);
    let baseline = store.usage().memory.committed();
    let max_bytes = page(1, 1, &[1, 2, 3, 4]).0.byte_len * 3;
    let descriptor = accumulated(AgeClock::Source, 4, 200e-9, max_bytes);
    let mut model: Vec<(u64, u64, u64, u64)> = vec![];
    let mut base = None;
    let mut watermark = 0;
    let mut random = 17u64;
    let mut evicted = 0;
    let mut dropped = 0;
    for key in 1..=256 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let time = u64::MAX - 1000 + random % 1000;
        let count = 1 + (random >> 32) % 4;
        let ids: Vec<_> = (0..count).collect();
        let bytes = page(key, 1, &ids).0.byte_len;
        let next = stage(
            &mut store,
            lease,
            key,
            &descriptor,
            base,
            if base.is_some() {
                CommitMode::Delta
            } else {
                CommitMode::Replace
            },
            &[(captured(key, 0.0, time, count), ids)],
        );
        base = Some(commit(&mut store, &next, 0));
        watermark = watermark.max(time);
        model.push((key, time, count, bytes));
        model.sort_by_key(|item| (item.1, item.0));
        while !model.is_empty()
            && (watermark - model[0].1 >= 200
                || model.len() > 4
                || model.iter().map(|item| item.3).sum::<u64>() > max_bytes)
        {
            let removed = model.remove(0);
            if removed.0 == key {
                dropped += 1;
            } else {
                evicted += 1;
            }
        }
        let current = retained(&store, &descriptor.key);
        let mut expected: Vec<_> = model.iter().map(|item| item.0).collect();
        expected.sort_unstable();
        assert_eq!(keys(&current), expected, "submission {key}");
        assert_eq!(
            current.counts().source_samples,
            model.iter().map(|item| item.2).sum::<u64>()
        );
        assert_eq!(
            store.usage().memory.page_bytes,
            model.iter().map(|item| item.3).sum::<u64>()
        );
        assert_eq!(store.counters().evicted_scans, evicted);
        assert_eq!(store.counters().dropped_scans, dropped);
        assert!(store.usage().memory.committed() <= store.limits().cpu_bytes);
    }
    let clear = DataRequest {
        lease,
        ..req(data_op::CLEAR)
    }
    .with_metadata(&NamespaceFilter {
        namespace: "mapping".into(),
    })
    .unwrap();
    ok(&mut store, &clear, 0);
    assert_eq!(store.usage().memory.committed(), baseline);
}

#[test]
fn expiry_retries_when_a_registry_permit_frees_even_while_simulation_is_paused() {
    let mut store = StreamingStore::new(
        EPOCH,
        DataLimits {
            transactions: 1,
            writer_transactions: 1,
            ..limits()
        },
    )
    .unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Simulation, 4, 1.0, 4096);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[(captured(1, 0.0, 1, 1), vec![1])],
    );
    commit(&mut store, &first, 0);
    let permit = store
        .registry()
        .lock()
        .unwrap()
        .begin_dataset(
            &DataKey::new("other", "reserved").unwrap(),
            DatasetKind::Cloud,
            None,
        )
        .unwrap();
    assert_eq!(
        store.advance_simulation(1.0).unwrap_err().code,
        data_code::BUDGET
    );
    let memory = store.usage().memory.committed();
    store.registry().lock().unwrap().cancel(permit);
    assert_eq!(store.usage().memory.committed(), memory);
    store.advance_simulation(1.0).unwrap();
    assert!(keys(&retained(&store, &descriptor.key)).is_empty());
    assert_eq!(store.counters().evicted_scans, 1);
}

#[test]
fn a_new_transaction_failure_is_not_hidden_by_an_older_maintenance_failure() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let descriptor = accumulated(AgeClock::Simulation, 4, 1.0, 4096);
    let first = stage(
        &mut store,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        &[(captured(1, 0.0, 1, 1), vec![1])],
    );
    let base = commit(&mut store, &first, 0);
    let usage = store.usage().memory;
    let pressure = store
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    assert!(store.advance_simulation(1.0).is_err());
    drop(pressure);
    let missing = begin(
        lease,
        2,
        descriptor.clone(),
        CommitMode::Delta,
        Some(base),
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &missing, 0);
    assert_eq!(
        call(&mut store, &edit(&missing, data_op::COMMIT), 0).code,
        data_code::INVALID
    );
    let status =
        DatasetStatus::from_json(&ok(&mut store, &status_request(&descriptor.key), 0).metadata)
            .unwrap();
    let failure = status.failure.unwrap();
    assert_eq!(failure.code, data_code::INVALID);
    assert_eq!(failure.phase, DataPhase::Validating);
    store.advance_simulation(1.0).unwrap();
    assert!(keys(&retained(&store, &descriptor.key)).is_empty());
}
