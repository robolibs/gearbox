use super::*;
use gearbox_api::{DataAdmission, DataPipelineConfig, DataPipelineError, DrawDataPipeline};

fn pipeline(host: &Harness, slots: usize) -> DrawDataPipeline {
    host.client
        .draw_data_pipeline(
            DataPipelineConfig {
                parallel_requests: 1,
                max_operations: slots,
                ..DataPipelineConfig::default()
            },
            end(),
        )
        .unwrap()
}

fn end() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn until(mut ready: impl FnMut() -> bool) {
    let deadline = end();
    while !ready() {
        assert!(Instant::now() < deadline, "wait condition timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn preflight_failures_submit_nothing_and_charge_nothing() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 0);
    let host = Harness::new(backend);
    let pipeline = pipeline(&host, 1);
    let baseline = pipeline.stats();
    assert_eq!(
        pipeline.wait_visible(&key, revision, None, Instant::now(), None),
        Err(DataPipelineError::Deadline {
            may_have_been_sent: false
        })
    );
    let token = AtomicBool::new(true);
    assert_eq!(
        pipeline.wait_visible(&key, revision, None, end(), Some(&token)),
        Err(DataPipelineError::Cancelled)
    );
    assert!(matches!(
        pipeline.wait_visible(
            &key,
            DatasetRevision {
                generation: 0,
                ..revision
            },
            None,
            end(),
            None
        ),
        Err(DataPipelineError::Protocol(_))
    ));
    assert!(matches!(
        pipeline.wait_visible(
            &key,
            revision,
            Some(ViewIdentity { id: 0, ..A }),
            end(),
            None
        ),
        Err(DataPipelineError::Protocol(_))
    ));
    assert_eq!(pipeline.stats(), baseline);
    pipeline.shutdown();
    assert_eq!(
        pipeline.wait_visible(&key, revision, None, end(), None),
        Err(DataPipelineError::Closed)
    );
    assert!(host.requests.lock().unwrap().is_empty());
}

#[test]
fn pending_timeout_is_read_only_frozen_and_recovers_for_historical_receipts() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 2);
    let before = backend.registry().lock().unwrap().revision();
    backend
        .complete_render_frame(&key, receipt(revision, B, 7, 2))
        .unwrap();
    let host = Harness::new(backend.clone());
    let pipeline = pipeline(&host, 2);
    let baseline = pipeline.stats().charged_bytes;
    assert!(matches!(
        pipeline.wait_visible(
            &key,
            revision,
            None,
            Instant::now() + Duration::from_millis(180),
            None
        ),
        Err(DataPipelineError::Deadline { .. })
    ));
    until(|| pipeline.stats().charged_bytes == baseline);
    assert_eq!(backend.registry().lock().unwrap().revision(), before);
    {
        let requests = host.requests.lock().unwrap();
        assert!(requests.len() >= 2);
        for (index, request) in requests.iter().enumerate() {
            assert_eq!(request.operation, data_op::STATUS);
            assert_eq!(
                (request.incarnation, request.base_generation),
                (revision.incarnation, revision.generation)
            );
            let query: DataStatusQuery = serde_json::from_slice(&request.metadata).unwrap();
            assert_eq!(query.view, if index == 0 { None } else { Some(A) });
        }
    }
    let completed = receipt(revision, A, 8, 2);
    backend.complete_render_frame(&key, completed).unwrap();
    let (_, second) = publish(&backend, Some(revision), 0);
    assert_eq!(
        pipeline
            .wait_visible(&key, revision, Some(A), end(), None)
            .unwrap(),
        completed
    );
    let empty = receipt(second, A, 9, 0);
    backend.complete_render_frame(&key, empty).unwrap();
    assert_eq!(
        pipeline
            .wait_visible(&key, second, None, end(), None)
            .unwrap(),
        empty
    );
    let (_, third) = publish(&backend, Some(second), 2);
    let culled = RenderCompletion {
        disposition: FrontierDisposition::Culled,
        selected_items: 0,
        ..receipt(third, A, 10, 2)
    };
    backend.complete_render_frame(&key, culled).unwrap();
    assert_eq!(
        pipeline
            .wait_visible(&key, third, None, end(), None)
            .unwrap(),
        culled
    );
    let (_, fourth) = publish(&backend, Some(third), 0);
    backend
        .complete_render_frame(&key, receipt(fourth, A, 11, 0))
        .unwrap();
    assert!(matches!(
        pipeline.wait_visible(&key, revision, Some(A), end(), None),
        Err(DataPipelineError::Visibility(
            GenerationVisibility::HistoryExpired { .. }
        ))
    ));
    until(|| pipeline.stats().charged_bytes == baseline);
    assert_eq!(pipeline.stats().outstanding, 0);
}

#[test]
fn terminal_states_preserve_targets_reasons_and_retention() {
    let backend = backend();
    let (key, first) = publish(&backend, None, 0);
    let (_, second) = publish(&backend, Some(first), 0);
    let host = Harness::new(backend.clone());
    let pipeline = pipeline(&host, 1);
    assert_eq!(
        pipeline.wait_visible(&key, first, Some(A), end(), None),
        Err(DataPipelineError::Visibility(
            GenerationVisibility::Superseded {
                target: VisibilityTarget {
                    dataset: first,
                    view: A
                },
                successor: second
            }
        ))
    );
    let failure = DataFailure {
        phase: DataPhase::Preparing,
        code: data_code::BUDGET,
        reason: "test pressure".into(),
    };
    backend
        .report_render_failure(&key, second, &failure)
        .unwrap();
    assert_eq!(
        pipeline.wait_visible(&key, second, Some(A), end(), None),
        Err(DataPipelineError::Visibility(
            GenerationVisibility::Failed {
                target: VisibilityTarget {
                    dataset: second,
                    view: A
                },
                failure
            }
        ))
    );
    backend.sync_render_views(&[B], Some(B)).unwrap();
    assert!(matches!(
        pipeline.wait_visible(&key, second, Some(A), end(), None),
        Err(DataPipelineError::Visibility(
            GenerationVisibility::ViewGone { .. }
        ))
    ));
    backend.sync_render_views(&[], None).unwrap();
    assert!(matches!(
        pipeline.wait_visible(&key, second, None, end(), None),
        Err(DataPipelineError::Visibility(
            GenerationVisibility::Unavailable { .. }
        ))
    ));
    assert!(
        host.requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.operation == data_op::STATUS)
    );
    assert_eq!(backend.registry().lock().unwrap().dataset_len(), 1);
}

#[test]
fn full_operation_slots_allow_cancellation_deadline_and_shutdown_without_submitting() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 0);
    let host = Harness::new(backend);
    let pipeline = pipeline(&host, 1);
    let baseline = pipeline.stats().charged_bytes;
    let request = DataRequest {
        host_epoch: pipeline.host_epoch(),
        ..DataRequest::new(data_op::STATUS)
    }
    .with_key(&key)
    .unwrap()
    .with_metadata(&DataStatusQuery::default())
    .unwrap();
    let mut held = pipeline
        .submit(&request, end(), DataAdmission::Reject)
        .unwrap();
    until(|| pipeline.stats().finished == 1);
    assert_eq!(
        pipeline.wait_visible(
            &key,
            revision,
            None,
            Instant::now() + Duration::from_millis(60),
            None
        ),
        Err(DataPipelineError::Deadline {
            may_have_been_sent: false
        })
    );
    let cancelled = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let wait =
            scope.spawn(|| pipeline.wait_visible(&key, revision, None, end(), Some(&cancelled)));
        until(|| pipeline.stats().charged_bytes > baseline + 40_960);
        cancelled.store(true, Ordering::Release);
        assert_eq!(wait.join().unwrap(), Err(DataPipelineError::Cancelled));
    });
    std::thread::scope(|scope| {
        let wait = scope.spawn(|| pipeline.wait_visible(&key, revision, None, end(), None));
        until(|| pipeline.stats().charged_bytes > baseline + 40_960);
        pipeline.shutdown();
        assert_eq!(wait.join().unwrap(), Err(DataPipelineError::Closed));
    });
    assert_eq!(pipeline.stats().submitted, 1);
    held.wait(end()).unwrap();
    pipeline.close();
    assert_eq!(pipeline.stats().charged_bytes, baseline);
}

#[test]
fn concurrent_waiters_share_queue_and_release_all_scratch() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 2);
    let host = Harness::new(backend.clone());
    let pipeline = pipeline(&host, 2);
    let baseline = pipeline.stats().charged_bytes;
    let completed = receipt(revision, A, 3, 2);
    std::thread::scope(|scope| {
        let waits = (0..12)
            .map(|_| scope.spawn(|| pipeline.wait_visible(&key, revision, None, end(), None)))
            .collect::<Vec<_>>();
        until(|| pipeline.stats().submitted >= 2);
        backend.complete_render_frame(&key, completed).unwrap();
        for wait in waits {
            assert_eq!(wait.join().unwrap().unwrap(), completed);
        }
    });
    pipeline.close();
    assert_eq!(pipeline.stats().outstanding, 0);
    assert_eq!(pipeline.stats().charged_bytes, baseline);
    assert!(pipeline.stats().peak_bytes <= DataPipelineConfig::default().max_bytes);
    assert!(
        host.requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.operation == data_op::STATUS)
    );
}

#[test]
fn waiter_scratch_is_reserved_before_any_status_request() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 0);
    let host = Harness::new(backend);
    let pipeline = host
        .client
        .draw_data_pipeline(
            DataPipelineConfig {
                parallel_requests: 1,
                max_operations: 1,
                max_bytes: (2 << 20) + 65_536,
                ..DataPipelineConfig::default()
            },
            end(),
        )
        .unwrap();
    let before = pipeline.stats();
    assert_eq!(
        pipeline.wait_visible(&key, revision, None, end(), None),
        Err(DataPipelineError::Full)
    );
    assert_eq!(pipeline.stats(), before);
    assert!(host.requests.lock().unwrap().is_empty());
}
