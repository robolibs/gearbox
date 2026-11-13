use gearbox_api::draw_data::*;
use gearbox_api::wire::draw_data::*;

#[allow(dead_code)]
#[path = "support/draw_data_contracts.rs"]
mod contracts;

#[path = "support/draw_data_wait_visible.rs"]
mod public_client;

const A: ViewIdentity = ViewIdentity { id: 1, epoch: 1 };
const B: ViewIdentity = ViewIdentity { id: 2, epoch: 1 };

fn backend() -> DrawingBackend {
    let backend = DrawingBackend::new(DataLimits {
        datasets: 4,
        completion_history: 2,
        ..DataLimits::reference()
    })
    .unwrap();
    backend.enable_renderer_reports().unwrap();
    backend
        .enable_renderer_visibility(2, &[DatasetKind::Cloud])
        .unwrap();
    backend.sync_render_views(&[A, B], Some(A)).unwrap();
    backend
}

fn publish(
    backend: &DrawingBackend,
    base: Option<DatasetRevision>,
    count: u32,
) -> (DataKey, DatasetRevision) {
    let mut schema = contracts::schema();
    schema.attributes.truncate(2);
    let descriptor = DataDescriptor {
        stream: StreamMode::RetainedMap,
        shape: DataShape::Cloud {
            schema: schema.clone(),
        },
        ..contracts::cloud()
    };
    let key = descriptor.key.clone();
    let budget = backend.budget();
    let mut pages = Vec::new();
    let mut scans = Vec::new();
    if count != 0 {
        let bytes = CloudPage {
            count,
            columns: vec![
                ColumnData {
                    values: vec![0; count as usize * 24],
                    validity: vec![],
                },
                ColumnData {
                    values: (0..u64::from(count)).flat_map(u64::to_le_bytes).collect(),
                    validity: vec![],
                },
            ],
        }
        .encode(&schema)
        .unwrap();
        let mut upload = PageUpload::new(
            budget,
            PageDescriptor {
                address: PageAddress {
                    scan_key: 0,
                    page_id: 0,
                },
                encoding: PageEncoding::CloudColumnsV1,
                items: count,
                byte_len: bytes.len() as u64,
                crc32: page_crc32(&bytes),
                origin_m: [0.0; 3],
            },
        )
        .unwrap();
        upload.push(0, &bytes).unwrap();
        pages.push(upload.finish_cloud(&schema).unwrap().into_page());
        scans.push(
            ScanMetadata::new(
                budget,
                &ScanDescriptor {
                    source_count: count as u64,
                    organization: None,
                    frame: DataFrame::default(),
                    ..contracts::scan(0)
                },
            )
            .unwrap(),
        );
    }
    let generation = Generation::build(
        budget,
        DatasetMetadata::new(budget, &descriptor).unwrap(),
        &scans,
        &pages,
    )
    .unwrap();
    let registry = backend.registry();
    let mut registry = registry.lock().unwrap();
    let permit = registry
        .begin_dataset(&key, DatasetKind::Cloud, base)
        .unwrap();
    (
        key,
        registry
            .commit_dataset(
                permit,
                DatasetStorage::from_resident(budget, generation).unwrap(),
            )
            .unwrap(),
    )
}

fn receipt(
    revision: DatasetRevision,
    view: ViewIdentity,
    frame: u64,
    count: u64,
) -> RenderCompletion {
    RenderCompletion {
        target: VisibilityTarget {
            dataset: revision,
            view,
        },
        frame,
        disposition: if count == 0 {
            FrontierDisposition::Empty
        } else {
            FrontierDisposition::Drawn
        },
        source_samples: count,
        selected_items: count,
    }
}

#[test]
fn no_host_renderer_or_no_eligible_view_never_claims_visibility() {
    let disabled = DrawingBackend::default();
    let (key, revision) = publish(&disabled, None, 0);
    assert!(matches!(
        disabled.render_visibility(&key, revision, None),
        GenerationVisibility::Unavailable { .. }
    ));
    assert!(
        !disabled
            .complete_render_frame(&key, receipt(revision, A, 1, 0))
            .unwrap()
    );
    let backend = backend();
    let (key, revision) = publish(&backend, None, 0);
    backend.sync_render_views(&[], None).unwrap();
    assert!(matches!(
        backend.render_visibility(&key, revision, None),
        GenerationVisibility::Unavailable { .. }
    ));
    assert!(
        !backend
            .complete_render_frame(&key, receipt(revision, A, 1, 0))
            .unwrap()
    );
}

#[test]
fn exact_completed_frame_changes_status_headers_but_not_retention() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 2);
    let mut store = StreamingStore::with_backend(17, backend.clone()).unwrap();
    assert_eq!(store.capabilities().rendered, [DatasetKind::Cloud]);
    assert!(store.capabilities().features.per_view_visibility);
    let request = DataRequest {
        host_epoch: 17,
        ..DataRequest::new(data_op::STATUS)
    }
    .with_key(&key)
    .unwrap()
    .with_metadata(&DataStatusQuery::default())
    .unwrap();
    let before = store.handle(&request, 0);
    assert_eq!(before.stage, data_stage::RETAINED);
    assert_eq!(
        (before.view_id, before.view_epoch, before.visible_generation),
        (1, 1, 0)
    );
    assert!(matches!(
        DatasetStatus::from_json(&before.metadata)
            .unwrap()
            .visibility,
        GenerationVisibility::Pending { .. }
    ));
    let completion = receipt(revision, A, 23, 2);
    assert!(backend.complete_render_frame(&key, completion).unwrap());
    let after = store.handle(&request, 1);
    after.validate().unwrap();
    assert_eq!(after.stage, data_stage::VISIBLE);
    assert_eq!(after.stored_generation, revision.generation);
    assert_eq!(after.visible_generation, revision.generation);
    assert_eq!(
        DatasetStatus::from_json(&after.metadata)
            .unwrap()
            .visibility,
        completion.visibility()
    );
}

#[test]
fn three_rapid_commits_reject_delayed_completions_and_never_inherit_success() {
    let backend = backend();
    let (key, first) = publish(&backend, None, 2);
    let (_, second) = publish(&backend, Some(first), 2);
    let (_, third) = publish(&backend, Some(second), 2);
    for old in [first, second] {
        assert!(
            !backend
                .complete_render_frame(&key, receipt(old, A, 10, 2))
                .unwrap()
        );
        assert!(
            matches!(backend.render_visibility(&key, old, Some(A)), GenerationVisibility::Superseded { successor, .. } if successor == third)
        );
    }
    assert!(
        backend
            .complete_render_frame(&key, receipt(third, A, 11, 2))
            .unwrap()
    );
    assert!(matches!(
        backend.render_visibility(&key, first, Some(A)),
        GenerationVisibility::Superseded { .. }
    ));
}

#[test]
fn successful_history_expires_explicitly_and_new_success_does_not_overwrite_it() {
    let backend = backend();
    let (key, first) = publish(&backend, None, 2);
    let mut revision = first;
    for frame in 1..=4 {
        assert!(
            backend
                .complete_render_frame(&key, receipt(revision, A, frame, 2))
                .unwrap()
        );
        if frame < 4 {
            revision = publish(&backend, Some(revision), 2).1;
        }
    }
    assert!(matches!(
        backend.render_visibility(&key, first, Some(A)),
        GenerationVisibility::HistoryExpired { .. }
    ));
    let second = DatasetRevision {
        generation: first.generation + 1,
        ..first
    };
    assert!(matches!(
        backend.render_visibility(&key, second, Some(A)),
        GenerationVisibility::Visible { frame: 2, .. }
    ));
    assert!(
        !backend
            .complete_render_frame(&key, receipt(second, A, 12, 2))
            .unwrap()
    );
    assert!(matches!(
        backend.render_visibility(&key, first, Some(A)),
        GenerationVisibility::HistoryExpired { .. }
    ));
}

#[test]
fn two_views_have_independent_frontiers_and_history() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 2);
    let drawn = receipt(revision, A, 1, 2);
    let culled = RenderCompletion {
        disposition: FrontierDisposition::Culled,
        selected_items: 0,
        ..receipt(revision, B, 1, 2)
    };
    assert!(backend.complete_render_frame(&key, drawn).unwrap());
    assert!(matches!(
        backend.render_visibility(&key, revision, Some(B)),
        GenerationVisibility::Pending { .. }
    ));
    assert!(backend.complete_render_frame(&key, culled).unwrap());
    assert_eq!(
        backend.render_visibility(&key, revision, Some(A)),
        drawn.visibility()
    );
    assert_eq!(
        backend.render_visibility(&key, revision, Some(B)),
        culled.visibility()
    );
}

#[test]
fn replacement_view_and_clear_recreate_cannot_accept_old_callbacks() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 0);
    assert!(
        backend
            .complete_render_frame(&key, receipt(revision, A, 1, 0))
            .unwrap()
    );
    let next_view = ViewIdentity { epoch: 2, ..A };
    backend
        .sync_render_views(&[next_view, B], Some(next_view))
        .unwrap();
    assert!(matches!(
        backend.render_visibility(&key, revision, Some(A)),
        GenerationVisibility::ViewGone { .. }
    ));
    assert!(
        !backend
            .complete_render_frame(&key, receipt(revision, A, 2, 0))
            .unwrap()
    );
    assert!(
        matches!(backend.render_visibility(&key, revision, None), GenerationVisibility::Pending { target } if target.view == next_view)
    );
    backend.lock().clear(&key.namespace);
    let (_, recreated) = publish(&backend, None, 0);
    assert_ne!(revision.incarnation, recreated.incarnation);
    assert!(
        !backend
            .complete_render_frame(&key, receipt(revision, next_view, 3, 0))
            .unwrap()
    );
    assert!(matches!(
        backend.render_visibility(&key, recreated, Some(next_view)),
        GenerationVisibility::Pending { .. }
    ));
}

#[test]
fn invalid_counts_frames_and_duplicate_views_do_not_mutate_the_book() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 2);
    assert!(
        backend
            .complete_render_frame(&key, receipt(revision, A, 0, 2))
            .is_err()
    );
    assert!(
        backend
            .complete_render_frame(&key, receipt(revision, A, 1, 3))
            .is_err()
    );
    assert!(
        backend
            .complete_render_frame(
                &key,
                RenderCompletion {
                    disposition: FrontierDisposition::Empty,
                    selected_items: 0,
                    ..receipt(revision, A, 1, 2)
                }
            )
            .is_err()
    );
    assert!(backend.sync_render_views(&[A, A], Some(A)).is_err());
    assert!(
        backend
            .sync_render_views(&[A, B, ViewIdentity { id: 3, epoch: 1 }], Some(A))
            .is_err()
    );
    assert!(
        backend
            .complete_render_frame(&key, receipt(revision, A, 3, 2))
            .unwrap()
    );
    assert!(
        !backend
            .complete_render_frame(&key, receipt(revision, A, 2, 2))
            .unwrap()
    );
}

#[test]
fn renderer_failure_is_view_specific_status_without_rejecting_commit() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 0);
    let failure = DataFailure {
        phase: DataPhase::Preparing,
        code: data_code::BUDGET,
        reason: "pressure".into(),
    };
    backend
        .report_render_failure(&key, revision, &failure)
        .unwrap();
    assert!(
        matches!(backend.render_visibility(&key, revision, Some(A)), GenerationVisibility::Failed { failure: value, .. } if value == failure)
    );
    backend.clear_render_failure(&key, revision, DataPhase::Preparing);
    assert!(
        backend
            .complete_render_frame(&key, receipt(revision, A, 3, 0))
            .unwrap()
    );
    assert!(matches!(
        backend.render_visibility(&key, revision, Some(A)),
        GenerationVisibility::Visible {
            disposition: FrontierDisposition::Empty,
            ..
        }
    ));
}

#[test]
fn bounded_visibility_metadata_is_reusable_and_released_on_backend_drop() {
    let backend = backend();
    let cpu = backend.budget().clone();
    let baseline = cpu.usage().committed();
    backend
        .enable_renderer_visibility(2, &[DatasetKind::Cloud])
        .unwrap();
    assert_eq!(cpu.usage().committed(), baseline);
    assert!(
        backend
            .enable_renderer_visibility(3, &[DatasetKind::Cloud])
            .is_err()
    );
    drop(backend);
    assert_eq!(cpu.usage().committed(), 0);
}
