use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use gearbox_api::draw_data::*;
use gearbox_api::wire::draw_data::*;

#[allow(dead_code)]
#[path = "support/draw_data_contracts.rs"]
mod contracts;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct Counted;

fn count() {
    let _ = ALLOCATIONS.try_with(|value| {
        if let Some(count) = value.get() {
            value.set(Some(count + 1));
        }
    });
}

unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counted = Counted;

fn measured<T>(f: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATIONS.with(|value| value.set(Some(0)));
    let result = f();
    let count = ALLOCATIONS.with(|value| value.replace(None).unwrap());
    (result, count)
}

fn backend() -> DrawingBackend {
    let backend = DrawingBackend::new(DataLimits {
        datasets: 8,
        ..DataLimits::reference()
    })
    .unwrap();
    backend.enable_renderer_reports().unwrap();
    backend
}

fn publish(
    backend: &DrawingBackend,
    key: &DataKey,
    base: Option<DatasetRevision>,
) -> DatasetRevision {
    let mut descriptor = contracts::cloud();
    descriptor.key = key.clone();
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
        .begin_dataset(key, DatasetKind::Cloud, base)
        .unwrap();
    registry
        .commit_dataset(
            permit,
            DatasetStorage::from_resident(backend.budget(), generation).unwrap(),
        )
        .unwrap()
}

#[test]
fn valid_reports_reads_replacement_and_clear_allocate_nothing_at_full_budget() {
    let backend = backend();
    let key = DataKey::new(
        &format!("{}a", "é".repeat(127)),
        &format!("{}b", "界".repeat(42)),
    )
    .unwrap();
    let revision = publish(&backend, &key, None);
    let failure = DataFailure {
        phase: DataPhase::Preparing,
        code: data_code::BUDGET,
        reason: "é".repeat(256),
    };
    let (_, control) = measured(|| failure.clone());
    assert!(
        control > 0,
        "allocator oracle must detect owned reason copies"
    );
    let usage = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    let (result, allocations) = measured(|| {
        let diagnostic =
            RenderDiagnostic::new(failure.phase, failure.code, &failure.reason).unwrap();
        let first = backend
            .report_render_failure(&key, revision, &failure)
            .unwrap();
        let read = backend.renderer_diagnostic(&key, revision);
        backend.clear_render_failure(&key, revision, DataPhase::Rendering);
        let preserved = backend.renderer_diagnostic(&key, revision) == Some(diagnostic);
        let replaced = backend
            .report_renderer_diagnostic(&key, revision, diagnostic)
            .unwrap();
        backend.clear_render_failure(&key, revision, DataPhase::Preparing);
        (
            first,
            read,
            preserved,
            replaced,
            backend.renderer_diagnostic(&key, revision),
        )
    });
    assert_eq!(allocations, 0);
    assert!(result.0 && result.2 && result.3);
    assert_eq!(result.1.unwrap().reason(), failure.reason);
    assert!(result.4.is_none());
    assert_eq!(backend.budget().usage().allocated, usage.allocated);
    assert_eq!(backend.budget().usage().committed(), usage.limit);
    drop(pressure);
}

#[test]
fn revision_pruning_and_neighbor_preservation_allocate_nothing() {
    let backend = backend();
    let key = DataKey::new("root", "cloud").unwrap();
    let neighbor = DataKey::new("root/child", "cloud").unwrap();
    let old = publish(&backend, &key, None);
    let other = publish(&backend, &neighbor, None);
    let diagnostic =
        RenderDiagnostic::new(DataPhase::Rendering, data_code::BUDGET, "GPU staging full").unwrap();
    backend
        .report_renderer_diagnostic(&key, old, diagnostic)
        .unwrap();
    backend
        .report_renderer_diagnostic(&neighbor, other, diagnostic)
        .unwrap();
    let next = publish(&backend, &key, Some(old));
    let usage = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    let ((stale, accepted, gone, live), allocations) = measured(|| {
        let stale = backend
            .report_renderer_diagnostic(&key, old, diagnostic)
            .unwrap();
        let accepted = backend
            .report_renderer_diagnostic(&key, next, diagnostic)
            .unwrap();
        (
            stale,
            accepted,
            backend.renderer_diagnostic(&key, old),
            backend.renderer_diagnostic(&neighbor, other),
        )
    });
    assert_eq!(allocations, 0);
    assert!(!stale && accepted);
    assert!(gone.is_none());
    assert_eq!(live, Some(diagnostic));
    assert_eq!(backend.budget().usage().allocated, usage.allocated);
    drop(pressure);
}

#[test]
fn borrowed_registry_keys_preserve_tuple_ordering_without_allocation() {
    let mut registry = DrawingStore::new(16, 16).unwrap();
    let keys: Vec<_> = [
        ("", "root"),
        ("a", "x"),
        ("a", "xx"),
        ("a/child", "x"),
        ("ab", "x"),
        ("é", "界"),
        ("界", "é"),
    ]
    .into_iter()
    .map(|(namespace, id)| DataKey::new(namespace, id).unwrap())
    .collect();
    let mut revisions = Vec::new();
    for key in &keys {
        let permit = registry
            .begin_dataset(key, DatasetKind::Cloud, None)
            .unwrap();
        revisions.push(registry.commit_dataset(permit, Arc::new(())).unwrap());
    }
    let (matches, allocations) = measured(|| {
        keys.iter().zip(&revisions).all(|(key, revision)| {
            registry
                .get_parts(&key.namespace, &key.id)
                .is_some_and(|entry| entry.revision == *revision)
        }) && [("a", "missing"), ("aa", "x"), ("a/child/next", "x")]
            .iter()
            .all(|(namespace, id)| registry.get_parts(namespace, id).is_none())
    });
    assert!(matches);
    assert_eq!(allocations, 0);
}

#[test]
fn invalid_diagnostics_do_not_mutate_reports_or_change_wire_payloads() {
    let backend = backend();
    let key = DataKey::new("validation", "cloud").unwrap();
    let revision = publish(&backend, &key, None);
    let expected = DataFailure {
        phase: DataPhase::Rendering,
        code: data_code::UNSUPPORTED,
        reason: "limits for \"café\" / 界".into(),
    };
    backend
        .report_render_failure(&key, revision, &expected)
        .unwrap();
    for (phase, code, reason) in [
        (DataPhase::Rendering, data_code::BUDGET, "x".repeat(513)),
        (DataPhase::Rendering, data_code::BUDGET, "".into()),
        (DataPhase::Rendering, data_code::OK, "bad code".into()),
        (
            DataPhase::Rendering,
            data_code::STALE_CURSOR + 1,
            "unknown code".into(),
        ),
        (
            DataPhase::Receiving,
            data_code::BUDGET,
            "wrong phase".into(),
        ),
        (
            DataPhase::Rendering,
            data_code::BUDGET,
            "embedded\0null".into(),
        ),
    ] {
        assert!(RenderDiagnostic::new(phase, code, &reason).is_err());
        assert!(
            backend
                .report_render_failure(
                    &key,
                    revision,
                    &DataFailure {
                        phase,
                        code,
                        reason
                    }
                )
                .is_err()
        );
        assert_eq!(
            backend.render_failure(&key, revision),
            Some(expected.clone())
        );
    }
    let actual = backend.render_failure(&key, revision).unwrap();
    assert_eq!(
        serde_json::to_vec(&actual).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
}

#[test]
fn inline_report_book_is_charged_once_and_released_with_backend() {
    let backend = backend();
    let budget = backend.budget().clone();
    let before = budget.usage().allocated;
    backend.enable_renderer_reports().unwrap();
    assert_eq!(budget.usage().allocated, before);
    let key = DataKey::new("report", "cloud").unwrap();
    let revision = publish(&backend, &key, None);
    let diagnostic =
        RenderDiagnostic::new(DataPhase::Preparing, data_code::BUDGET, "CPU full").unwrap();
    let before = budget.usage().allocated;
    backend
        .report_renderer_diagnostic(&key, revision, diagnostic)
        .unwrap();
    assert_eq!(budget.usage().allocated, before);
    drop(backend);
    assert_eq!(budget.usage().committed(), 0);
}
