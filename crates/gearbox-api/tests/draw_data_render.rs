use std::sync::{Arc, Barrier};

use gearbox_api::draw_data::*;
use gearbox_api::wire::draw_data::*;

#[allow(dead_code)]
#[path = "support/draw_data_contracts.rs"]
mod contracts;

fn backend() -> DrawingBackend {
    DrawingBackend::new(DataLimits {
        datasets: 8,
        gpu_bytes: 64 * 1024,
        upload_bytes_per_frame: 32 * 1024,
        ..DataLimits::reference()
    })
    .unwrap()
}

fn publish(backend: &DrawingBackend, base: Option<DatasetRevision>) -> (DataKey, DatasetRevision) {
    let mut descriptor = contracts::cloud();
    descriptor.stream = StreamMode::RetainedMap;
    let key = descriptor.key.clone();
    let generation = Generation::build(
        backend.budget(),
        DatasetMetadata::new(backend.budget(), &descriptor).unwrap(),
        &[],
        &[],
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
                DatasetStorage::from_resident(backend.budget(), generation).unwrap(),
            )
            .unwrap(),
    )
}

fn failure() -> DataFailure {
    DataFailure {
        phase: DataPhase::Preparing,
        code: data_code::BUDGET,
        reason: "prepared records exceed the configured budget".into(),
    }
}

#[test]
fn retirement_marker_remains_charged_until_last_reader_and_clear_needs_no_headroom() {
    let backend = backend();
    let baseline = backend.budget().usage().allocated;
    let (key, _) = publish(&backend, None);
    let registry = backend.registry();
    let mut registry = registry.lock().unwrap();
    let marker = registry
        .get(&(key.namespace.clone(), key.id.clone()))
        .unwrap()
        .lifetime
        .clone();
    let usage = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    assert!(backend.budget().reserve(1).is_err());
    assert_eq!(registry.clear(&key.namespace), Ok(1));
    assert!(!marker.is_live());
    while let Some(retired) = registry.pop_retired() {
        drop(retired);
    }
    assert!(backend.budget().usage().allocated > baseline);
    drop(marker);
    assert_eq!(backend.budget().usage().allocated, baseline);
    drop(pressure);
    assert_eq!(backend.budget().usage().committed(), baseline);
}

#[test]
fn exhausted_identity_admission_preserves_existing_incarnation_and_consumes_failed_permit() {
    let backend = backend();
    let (key, first) = publish(&backend, None);
    let mut descriptor = contracts::cloud();
    descriptor.key.id.push_str("-new");
    let new_key = descriptor.key.clone();
    let new_data = DatasetStorage::from_resident(
        backend.budget(),
        Generation::build(
            backend.budget(),
            DatasetMetadata::new(backend.budget(), &descriptor).unwrap(),
            &[],
            &[],
        )
        .unwrap(),
    )
    .unwrap();
    let registry = backend.registry();
    let mut registry = registry.lock().unwrap();
    let entry = registry
        .get(&(key.namespace.clone(), key.id.clone()))
        .unwrap();
    let old_data = entry.payload.clone();
    let marker = entry.lifetime.clone();
    let update = registry
        .begin_dataset(&key, DatasetKind::Cloud, Some(first))
        .unwrap();
    let create = registry
        .begin_dataset(&new_key, DatasetKind::Cloud, None)
        .unwrap();
    let usage = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    let next = registry.commit_dataset(update, old_data).unwrap();
    assert_eq!(next.incarnation, first.incarnation);
    assert_eq!(next.generation, first.generation + 1);
    assert!(marker.is_live());
    assert_eq!(
        registry.commit_dataset(create, new_data),
        Err(IdentityError::Budget)
    );
    assert_eq!(registry.pending_len(), 0);
    assert_eq!(registry.dataset_len(), 1);
    assert!(!registry.permit_is_live(create));
    assert!(registry.get(&(new_key.namespace, new_key.id)).is_none());
    assert_eq!(
        registry.get(&(key.namespace, key.id)).unwrap().revision,
        next
    );
    drop(pressure);
}

fn status(
    store: &mut StreamingStore,
    key: &DataKey,
    generation: Option<u64>,
) -> (DataReply, DatasetStatus) {
    let request = DataRequest {
        host_epoch: 17,
        ..DataRequest::new(data_op::STATUS)
    }
    .with_key(key)
    .unwrap()
    .with_metadata(&DataStatusQuery {
        generation,
        view: None,
    })
    .unwrap();
    let reply = store.handle(&request, 1);
    assert_eq!(
        reply.code,
        data_code::OK,
        "{}",
        String::from_utf8_lossy(&reply.message)
    );
    let status = DatasetStatus::from_json(&reply.metadata).unwrap();
    (reply, status)
}

#[test]
fn gpu_charges_cover_overlap_staging_and_deferred_owners() {
    let backend = backend();
    let gpu = backend.gpu().clone();
    let old = gpu.reserve(32 * 1024).unwrap();
    let new = gpu.reserve(16 * 1024).unwrap();
    let staging = gpu.reserve(16 * 1024).unwrap();
    assert_eq!(gpu.usage().bytes, gpu.limit());
    assert!(gpu.reserve(1).is_none());
    assert!(gpu.reserve(u64::MAX).is_none());
    gpu.record_upload(staging.bytes());
    let retire = move || drop((old, staging));
    assert_eq!(gpu.usage().bytes, 64 * 1024);
    retire();
    assert_eq!(
        gpu.usage(),
        GpuUsage {
            bytes: 16 * 1024,
            peak: 64 * 1024,
            uploaded: 16 * 1024,
            other: 16 * 1024,
            ..Default::default()
        }
    );
    drop(new);
    assert_eq!(gpu.usage().bytes, 0);
}

#[test]
fn classified_gpu_allocations_and_retirement_share_one_atomic_total() {
    let backend = backend();
    let gpu = backend.gpu();
    let mut records = gpu.reserve_class(8192, GpuClass::Records).unwrap();
    let uniforms = gpu.reserve_class(128, GpuClass::Uniforms).unwrap();
    let staging = gpu.reserve_class(4096, GpuClass::Staging).unwrap();
    let auxiliary = gpu.reserve_class(32768, GpuClass::Auxiliary).unwrap();
    records.retire();
    records.retire();
    assert_eq!(
        gpu.usage(),
        GpuUsage {
            bytes: 45184,
            peak: 45184,
            records: 8192,
            uniforms: 128,
            staging: 4096,
            auxiliary: 32768,
            retiring: 8192,
            ..Default::default()
        }
    );
    assert!(gpu.reserve_class(20353, GpuClass::Auxiliary).is_none());
    assert!(gpu.reserve_class(u64::MAX, GpuClass::Staging).is_none());
    drop((records, staging));
    assert_eq!(gpu.usage().retiring, 0);
    assert_eq!(gpu.usage().bytes, 32896);
    assert_eq!(gpu.usage().records, 0);
    assert_eq!(gpu.usage().staging, 0);
    drop((uniforms, auxiliary));
    assert_eq!(gpu.usage().bytes, 0);
    assert_eq!(gpu.usage().auxiliary, 0);
}

#[test]
fn concurrent_gpu_owners_share_one_cap_and_release_to_zero() {
    let backend = backend();
    let held = Arc::new(Barrier::new(9));
    let release = Arc::new(Barrier::new(9));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let gpu = backend.gpu().clone();
            let held = held.clone();
            let release = release.clone();
            std::thread::spawn(move || {
                let charge = gpu.reserve(4096).unwrap();
                held.wait();
                release.wait();
                drop(charge);
            })
        })
        .collect();
    held.wait();
    assert_eq!(backend.gpu().usage().bytes, 32768);
    assert!(backend.gpu().reserve(32769).is_none());
    release.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(backend.gpu().usage().bytes, 0);
    assert_eq!(backend.gpu().usage().peak, 32768);
}

#[test]
fn gpu_ledger_metadata_lives_until_the_last_deferred_charge() {
    let backend = backend();
    let cpu = backend.budget().clone();
    let charge = backend.gpu().reserve(1024).unwrap();
    drop(backend);
    assert!(cpu.usage().committed() > 0);
    drop(charge);
    assert_eq!(cpu.usage().committed(), 0);
}

#[test]
fn api_status_reports_gpu_bytes_without_claiming_visibility() {
    let backend = backend();
    let (key, revision) = publish(&backend, None);
    let mut store = StreamingStore::with_backend(17, backend.clone()).unwrap();
    let allocation = backend.gpu().reserve(4096).unwrap();
    let staging = backend.gpu().reserve(128).unwrap();
    let (reply, data) = status(&mut store, &key, None);
    assert_eq!(reply.stage, data_stage::RETAINED);
    assert_eq!(data.retained, Some(revision));
    assert_eq!(data.resources.gpu_bytes, 4224);
    assert!(matches!(
        data.visibility,
        GenerationVisibility::Unavailable { .. }
    ));
    drop(staging);
    assert_eq!(status(&mut store, &key, None).1.resources.gpu_bytes, 4096);
    drop(allocation);
    assert_eq!(status(&mut store, &key, None).1.resources.gpu_bytes, 0);
    assert!(store.capabilities().rendered.is_empty());
}

#[test]
fn renderer_failure_is_observable_but_does_not_reject_a_retained_commit() {
    let backend = backend();
    backend.enable_renderer_reports().unwrap();
    let baseline = backend.budget().usage().committed();
    backend.enable_renderer_reports().unwrap();
    assert_eq!(backend.budget().usage().committed(), baseline);
    let (key, revision) = publish(&backend, None);
    let mut store = StreamingStore::with_backend(17, backend.clone()).unwrap();
    assert!(
        backend
            .report_render_failure(&key, revision, &failure())
            .unwrap()
    );
    let (reply, data) = status(&mut store, &key, None);
    assert_eq!(reply.stage, data_stage::RETAINED);
    assert_eq!(data.retained, Some(revision));
    assert_eq!(data.failure, Some(failure()));
    backend.clear_render_failure(&key, revision, DataPhase::Rendering);
    assert_eq!(status(&mut store, &key, None).1.failure, Some(failure()));
    backend.clear_render_failure(&key, revision, DataPhase::Preparing);
    assert!(status(&mut store, &key, None).1.failure.is_none());
}

#[test]
fn stale_reports_cannot_cross_replace_or_clear_recreate() {
    let backend = backend();
    backend.enable_renderer_reports().unwrap();
    let (key, old) = publish(&backend, None);
    let mut store = StreamingStore::with_backend(17, backend.clone()).unwrap();
    assert!(
        backend
            .report_render_failure(&key, old, &failure())
            .unwrap()
    );
    let (_, next) = publish(&backend, Some(old));
    assert!(
        !backend
            .report_render_failure(&key, old, &failure())
            .unwrap()
    );
    assert!(status(&mut store, &key, None).1.failure.is_none());
    assert!(
        backend
            .report_render_failure(&key, next, &failure())
            .unwrap()
    );
    assert!(
        status(&mut store, &key, Some(old.generation))
            .1
            .failure
            .is_none()
    );
    backend.lock().clear(&key.namespace);
    let (_, recreated) = publish(&backend, None);
    assert_ne!(recreated.incarnation, next.incarnation);
    assert!(
        !backend
            .report_render_failure(&key, next, &failure())
            .unwrap()
    );
    assert!(status(&mut store, &key, None).1.failure.is_none());
}

#[test]
fn render_report_text_and_phase_are_validated_before_state_changes() {
    let backend = backend();
    backend.enable_renderer_reports().unwrap();
    let (key, revision) = publish(&backend, None);
    let mut bad = failure();
    bad.reason = "x".repeat(513);
    assert!(backend.report_render_failure(&key, revision, &bad).is_err());
    bad = failure();
    bad.phase = DataPhase::Receiving;
    assert!(backend.report_render_failure(&key, revision, &bad).is_err());
    assert!(backend.render_failure(&key, revision).is_none());
}
