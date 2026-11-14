use super::*;
use gearbox_api::draw_data::{DatasetMetadata, Generation};
use gearbox_api::wire::draw_data::*;

#[path = "frame_cache_tests.rs"]
mod frame_cache;

fn lifetime() -> DatasetLifetime {
    let mut registry = gearbox_api::draw_data::DrawingStore::new(1, 1).unwrap();
    let key = DataKey::new("fixture", "lifetime").unwrap();
    let permit = registry
        .begin_dataset(&key, DatasetKind::Cloud, None)
        .unwrap();
    registry.commit_dataset(permit, Arc::new(())).unwrap();
    registry
        .get(&(key.namespace, key.id))
        .unwrap()
        .lifetime
        .clone()
}

pub(super) fn retained(budget: &CpuBudget) -> Retained {
    retained_points(budget, &[[0.0, 0.0, 1.0], [1.0, 2.0, 3.0]])
}

pub(super) fn retained_points(budget: &CpuBudget, points: &[[f64; 3]]) -> Retained {
    retained_paged(budget, points, points.len().max(1))
}

pub(super) fn retained_paged(
    budget: &CpuBudget,
    points: &[[f64; 3]],
    page_size: usize,
) -> Retained {
    let (schema, payload) = super::super::tests::fixture(points, vec![]);
    let pages: Vec<_> = if points.is_empty() {
        vec![super::super::tests::stored(
            budget, &schema, &payload, [0.0; 3],
        )]
    } else {
        points
            .chunks(page_size)
            .enumerate()
            .map(|(index, points)| {
                let (_, mut payload) = super::super::tests::fixture(points, vec![]);
                for (offset, id) in payload.columns[1].values.chunks_exact_mut(8).enumerate() {
                    id.copy_from_slice(
                        &(u64::MAX - (index * page_size + offset) as u64).to_le_bytes(),
                    );
                }
                super::super::tests::stored_address(
                    budget,
                    &schema,
                    &payload,
                    [0.0; 3],
                    index as u64,
                )
            })
            .collect()
    };
    let descriptor = DatasetMetadata::new(
        budget,
        &DataDescriptor {
            revision: SCHEMA_REVISION,
            key: DataKey::new("worker-test", "cloud").unwrap(),
            shape: DataShape::Cloud { schema },
            stream: StreamMode::RetainedMap,
            lifetime_s: None,
            presentation: Presentation::default(),
        },
    )
    .unwrap();
    let scan = ScanMetadata::new(
        budget,
        &ScanDescriptor {
            revision: SCHEMA_REVISION,
            scan_key: 0,
            source: None,
            frame: DataFrame::default(),
            source_count: points.len() as u64,
            organization: None,
        },
    )
    .unwrap();
    Retained {
        revision: DatasetRevision {
            incarnation: 1,
            generation: 1,
        },
        generation: DatasetStorage::from_resident(
            budget,
            Generation::build(budget, descriptor, &[scan], &pages).unwrap(),
        )
        .unwrap(),
        lifetime: lifetime(),
    }
}

#[test]
fn palette_and_frost_reuse_geometry_and_selected_records() {
    let budget = CpuBudget::new(16 * 1024 * 1024);
    let serial = AtomicU64::new(1);
    let entry = retained(&budget);
    let first = prepare_dataset(&budget, &entry, None, 1, &serial).unwrap();
    let mut presentation = entry.generation.descriptor().presentation.clone();
    presentation.color = ColorMode::Constant {
        rgba: [220, 60, 90, 255],
    };
    presentation.blur = 0.6;
    presentation.opacity = 0.8;
    let next = Retained {
        revision: DatasetRevision {
            generation: 2,
            ..entry.revision
        },
        generation: entry.generation.restyle(&budget, &presentation).unwrap(),
        lifetime: entry.lifetime.clone(),
    };
    let second = prepare_dataset(&budget, &next, Some(&first), 1, &serial).unwrap();
    assert!(Arc::ptr_eq(&first.pages[0], &second.pages[0]));
    assert!(std::ptr::eq(
        &first.pages[0].appearance.geometry.hierarchy,
        &second.pages[0].appearance.geometry.hierarchy,
    ));
    assert_eq!(
        first.alpha.required(first.pages[0].appearance.alpha, 4),
        super::super::alpha::OPAQUE
    );
    assert_eq!(
        second.alpha.required(second.pages[0].appearance.alpha, 4),
        super::super::alpha::TRANSMITTING
    );
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(second.render_pages(), 1);
    drop((first, second, entry, next));
    assert_eq!(budget.usage().allocated, 0);
}

#[test]
fn continuous_to_all_indexed_palettes_reuses_the_complete_prepared_page() {
    let budget = CpuBudget::new(16 << 20);
    let serial = AtomicU64::new(1);
    let mut entry = retained(&budget);
    let mut p = entry.generation.descriptor().presentation.clone();
    p.color = ColorMode::Axis {
        axis: 2,
        colormap: gearbox_api::colormap::VIRIDIS,
        range: [0.0, 4.0],
    };
    entry.generation = entry.generation.restyle(&budget, &p).unwrap();
    let first = prepare_dataset(&budget, &entry, None, 1, &serial).unwrap();
    for map in super::super::color_tests::MAPS {
        if let ColorMode::Axis { colormap, .. } = &mut p.color {
            *colormap = map;
        }
        p.blur = 0.7;
        let next = Retained {
            revision: DatasetRevision {
                generation: 2,
                ..entry.revision
            },
            generation: entry.generation.restyle(&budget, &p).unwrap(),
            lifetime: entry.lifetime.clone(),
        };
        let second = prepare_dataset(&budget, &next, Some(&first), 1, &serial).unwrap();
        assert!(Arc::ptr_eq(&first.pages[0], &second.pages[0]));
        assert!(Arc::ptr_eq(
            &first.generation.resident().unwrap().pages()[0],
            &second.generation.resident().unwrap().pages()[0]
        ));
        assert_eq!(
            second.pages[0].appearance.samples()[1].palette_indices,
            super::super::palette::indices(3.0)
        );
    }
    drop((first, entry));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn selector_change_reuses_positions_but_rebuilds_selected_values() {
    let budget = CpuBudget::new(16 * 1024 * 1024);
    let serial = AtomicU64::new(1);
    let entry = retained(&budget);
    let first = prepare_dataset(&budget, &entry, None, 1, &serial).unwrap();
    let mut presentation = entry.generation.descriptor().presentation.clone();
    presentation.color = ColorMode::Axis {
        axis: 2,
        colormap: gearbox_api::colormap::VIRIDIS,
        range: [0.0, 4.0],
    };
    let next = Retained {
        revision: DatasetRevision {
            generation: 2,
            ..entry.revision
        },
        generation: entry.generation.restyle(&budget, &presentation).unwrap(),
        lifetime: entry.lifetime.clone(),
    };
    let second = prepare_dataset(&budget, &next, Some(&first), 1, &serial).unwrap();
    assert!(Arc::ptr_eq(
        &first.pages[0].appearance.geometry,
        &second.pages[0].appearance.geometry
    ));
    assert!(!Arc::ptr_eq(
        &first.pages[0].appearance,
        &second.pages[0].appearance
    ));
    assert_eq!(second.pages[0].appearance.samples()[1].value, 0.75);
}

#[test]
fn clipping_changes_only_page_metadata_and_retains_original_measurements() {
    let budget = CpuBudget::new(16 << 20);
    let serial = AtomicU64::new(1);
    let entry = retained(&budget);
    let first = prepare_dataset(&budget, &entry, None, 1, &serial).unwrap();
    let mut presentation = entry.generation.descriptor().presentation.clone();
    presentation.clip = Some(ClipBounds {
        min_m: [-0.1, -0.1, 0.9],
        max_m: [0.1, 0.1, 1.1],
        ..Default::default()
    });
    let next = Retained {
        revision: DatasetRevision {
            generation: 2,
            ..entry.revision
        },
        generation: entry.generation.restyle(&budget, &presentation).unwrap(),
        lifetime: entry.lifetime.clone(),
    };
    let second = prepare_dataset(&budget, &next, Some(&first), 1, &serial).unwrap();
    assert!(!Arc::ptr_eq(&first.pages[0], &second.pages[0]));
    assert!(Arc::ptr_eq(
        &first.pages[0].appearance,
        &second.pages[0].appearance
    ));
    assert!(Arc::ptr_eq(
        &entry.generation.resident().unwrap().pages()[0],
        &next.generation.resident().unwrap().pages()[0]
    ));
    assert_eq!(first.pages[0].clip.selected, 2);
    assert_eq!(second.pages[0].clip.selected, 1);
    assert_eq!(second.generation.counts().valid_samples, 2);
    drop((first, second, entry, next));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn superseded_preparation_releases_temporary_reservations() {
    let budget = CpuBudget::new(16 * 1024 * 1024);
    let entry = retained(&budget);
    let baseline = budget.usage().allocated;
    assert!(prepare_dataset(&budget, &entry, None, 1, &AtomicU64::new(2)).is_err());
    assert_eq!(budget.usage().allocated, baseline);
    assert_eq!(budget.usage().reserved, 0);
}

#[test]
fn capture_is_nonblocking_and_clear_removes_incarnation() {
    let backend = DrawingBackend::default();
    let entry = retained(backend.budget());
    let registry = backend.registry();
    let mut lock = registry.lock().unwrap();
    let key = entry.generation.descriptor().key.clone();
    let permit = lock.begin_dataset(&key, DatasetKind::Cloud, None).unwrap();
    let revision = lock.commit_dataset(permit, entry.generation).unwrap();
    let mut seen = u64::MAX;
    assert!(Snapshot::capture(&backend, &mut seen).unwrap().is_none());
    assert_eq!(seen, u64::MAX);
    drop(lock);
    let snapshot = Snapshot::capture(&backend, &mut seen).unwrap().unwrap();
    assert!(snapshot.contains(&key, revision.incarnation));
    assert!(Snapshot::capture(&backend, &mut seen).unwrap().is_none());
    backend.lock().clear(&key.namespace);
    let cleared = Snapshot::capture(&backend, &mut seen).unwrap().unwrap();
    assert!(!cleared.contains(&key, revision.incarnation));
    assert!(!cleared.same(&snapshot));
}

#[test]
fn worker_keeps_only_latest_pending_snapshot_and_joins_cleanly() {
    let backend = DrawingBackend::default();
    let baseline = backend.budget().usage().allocated;
    let mut seen = u64::MAX;
    let snapshot = Snapshot::capture(&backend, &mut seen).unwrap().unwrap();
    let worker = Worker::new(backend.clone()).unwrap();
    let mut latest = 0;
    for _ in 0..100 {
        latest = worker.submit(snapshot.clone());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let scene = loop {
        if let Some(scene) = worker.take() {
            if scene.sequence == latest {
                break scene;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "worker did not finish latest snapshot"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    assert!(scene.datasets.is_empty());
    assert!(scene.failures.is_empty());
    drop((worker, snapshot, scene));
    assert_eq!(backend.budget().usage().allocated, baseline);
}

#[test]
fn recovered_poisoned_registry_does_not_disable_renderer_snapshots() {
    let backend = DrawingBackend::default();
    let registry = backend.registry();
    assert!(
        std::thread::spawn(move || {
            let _guard = registry.lock().unwrap();
            panic!("poison test");
        })
        .join()
        .is_err()
    );
    let mut seen = u64::MAX;
    let snapshot = Snapshot::capture(&backend, &mut seen).unwrap().unwrap();
    assert!(snapshot.entries.is_empty());
    assert_ne!(seen, u64::MAX);
}

fn published_snapshot(backend: &DrawingBackend) -> (Arc<Snapshot>, DataKey, DatasetRevision) {
    let entry = retained(backend.budget());
    let key = entry.generation.descriptor().key.clone();
    let registry = backend.registry();
    let mut registry = registry.lock().unwrap();
    let permit = registry
        .begin_dataset(&key, DatasetKind::Cloud, None)
        .unwrap();
    let revision = registry.commit_dataset(permit, entry.generation).unwrap();
    drop(registry);
    let mut seen = u64::MAX;
    (
        Snapshot::capture(backend, &mut seen).unwrap().unwrap(),
        key,
        revision,
    )
}

#[test]
fn source_count_above_visible_budget_reaches_selection_without_truncation() {
    let backend = DrawingBackend::new(DataLimits {
        visible_points: 1,
        ..Default::default()
    })
    .unwrap();
    let (snapshot, _, _) = published_snapshot(&backend);
    let scene = prepare_scene(&backend, 1, &snapshot, None, &AtomicU64::new(1), None).unwrap();
    assert!(scene.failures.is_empty());
    let datasets = scene.datasets.read();
    assert_eq!(datasets.len(), 1);
    assert_eq!(datasets[0].generation.counts().valid_samples, 2);
    assert_eq!(
        datasets[0].pages[0]
            .appearance
            .geometry
            .positions
            .values
            .len(),
        2
    );
    assert_eq!(backend.limits().visible_points, 1);
}

#[test]
fn exhausted_snapshot_replacement_still_revokes_clear_and_preserves_namespace_neighbors() {
    let backend = DrawingBackend::default();
    let (first, key, revision) = published_snapshot(&backend);
    let mut desc = first.entries.read()[0].generation.descriptor().clone();
    desc.key.namespace.push_str("-neighbor");
    let neighbor = desc.key.clone();
    let generation = DatasetStorage::from_resident(
        backend.budget(),
        Generation::build(
            backend.budget(),
            DatasetMetadata::new(backend.budget(), &desc).unwrap(),
            first.entries.read()[0].generation.scans(),
            first.entries.read()[0]
                .generation
                .resident()
                .unwrap()
                .pages(),
        )
        .unwrap(),
    )
    .unwrap();
    let neighbor_revision = {
        let registry = backend.registry();
        let mut registry = registry.lock().unwrap();
        let permit = registry
            .begin_dataset(&neighbor, DatasetKind::Cloud, None)
            .unwrap();
        registry.commit_dataset(permit, generation).unwrap()
    };
    let mut seen = u64::MAX;
    let snapshot = Snapshot::capture(&backend, &mut seen).unwrap().unwrap();
    let serial = AtomicU64::new(1);
    let scene = Arc::new(prepare_scene(&backend, 1, &snapshot, None, &serial, None).unwrap());
    let offered = super::super::frontier::Offered {
        snapshot: Some(snapshot.clone()),
        scene: Some(scene.clone()),
        retention: super::super::frontier::Retention::new(backend.budget()).unwrap(),
    };
    let usage = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    assert_eq!(backend.lock().clear(&key.namespace), 1);
    assert!(matches!(
        Snapshot::capture(&backend, &mut seen),
        Err(StorageError::Budget { .. })
    ));
    assert!(!snapshot.contains(&key, revision.incarnation));
    assert!(snapshot.contains(&neighbor, neighbor_revision.incarnation));
    let held = offered.retention.read();
    let mut needed = Vec::new();
    offered.visit_needed(&held, |data| needed.push(data.key().clone()));
    assert_eq!(needed, [neighbor.clone()]);
    assert_eq!(snapshot.entries.prune(), 1);
    assert_eq!(snapshot.entries.read().capacity(), 2);
    drop(pressure);
    let fresh = Snapshot::capture(&backend, &mut seen).unwrap().unwrap();
    assert_eq!(fresh.entries.len(), 1);
    assert_eq!(fresh.entries.read().capacity(), 1);
    assert!(!snapshot.same(&fresh));
    assert!(fresh.contains(&neighbor, neighbor_revision.incarnation));
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < until, "worker deadline elapsed");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn reap_namespace(backend: &DrawingBackend, namespace: &str) {
    let registry = backend.registry();
    let mut registry = registry.lock().unwrap();
    registry.clear(namespace).unwrap();
    while let Some(retired) = registry.pop_retired() {
        drop(retired);
    }
}

#[test]
fn retired_preparation_cancels_before_admission_even_without_sequence_change() {
    let backend = DrawingBackend::default();
    let (snapshot, key, _) = published_snapshot(&backend);
    let serial = AtomicU64::new(1);
    let entries = snapshot.entries.read();
    let entry = &entries[0];
    assert!(check_preparation(entry, 1, &serial).is_ok());
    reap_namespace(&backend, &key.namespace);
    let usage = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    let error = prepare_dataset(backend.budget(), entry, None, 1, &serial)
        .err()
        .expect("retired preparation must fail");
    assert_eq!(error.to_string(), invalid("dataset retired").to_string());
    assert_eq!(serial.load(Ordering::Acquire), 1);
    assert_eq!(backend.budget().usage().allocated, usage.allocated);
    assert_eq!(backend.budget().usage().committed(), usage.limit);
    drop(pressure);
}

#[test]
fn retired_empty_preparation_is_not_published() {
    let backend = DrawingBackend::default();
    let (snapshot, key, _) = published_snapshot(&backend);
    let serial = AtomicU64::new(1);
    let entries = snapshot.entries.read();
    let entry = Retained {
        revision: entries[0].revision,
        lifetime: entries[0].lifetime.clone(),
        generation: DatasetStorage::from_resident(
            backend.budget(),
            Generation::build(
                backend.budget(),
                entries[0].generation.metadata().clone(),
                &[],
                &[],
            )
            .unwrap(),
        )
        .unwrap(),
    };
    assert!(prepare_dataset(backend.budget(), &entry, None, 1, &serial).is_ok());
    reap_namespace(&backend, &key.namespace);
    assert!(prepare_dataset(backend.budget(), &entry, None, 1, &serial).is_err());
    let scene = prepare_scene(&backend, 1, &snapshot, None, &serial, None).unwrap();
    assert!(scene.datasets.is_empty());
    assert!(scene.failures.is_empty());
    assert!(!scene.retryable);
}

#[test]
fn retired_payloads_leave_shared_snapshot_scene_and_view_roots_without_new_allocation() {
    use super::super::frontier::{DatasetSet, Offered, Retention, Selection, ViewFrontier};
    use bevy::{prelude::Entity, render::view::RetainedViewEntity};
    let backend = DrawingBackend::default();
    let worker = Worker::new(backend.clone()).unwrap();
    let (snapshot, key, _) = published_snapshot(&backend);
    let generation = Arc::downgrade(&snapshot.entries.read()[0].generation);
    worker.submit(snapshot.clone());
    let mut finished = None;
    wait_until(|| {
        finished = worker.take();
        finished.is_some()
    });
    let scene = finished.unwrap();
    let dataset = Arc::downgrade(&scene.datasets.read()[0]);
    let set = DatasetSet::select(
        backend.budget(),
        None,
        &scene.datasets.read(),
        |_| true,
        |_| true,
    )
    .unwrap();
    let mut selection = Selection::new(backend.budget(), 2).unwrap();
    for id in 1..=2 {
        selection.views.push(ViewFrontier {
            retained: RetainedViewEntity::new(Entity::from_raw_u32(id).unwrap().into(), None, 0),
            identity: ViewIdentity {
                id: id as u64,
                epoch: 1,
            },
            datasets: set.clone(),
            preparing: None,
        });
    }
    let offered = Offered {
        snapshot: Some(snapshot),
        scene: Some(scene.clone()),
        retention: Retention::new(backend.budget()).unwrap(),
    };
    offered.retention.select(Some(Arc::new(selection)));
    let extracted =
        <Offered as bevy::render::extract_resource::ExtractResource<bevy::render::RenderApp>>::extract_resource(&offered);
    let reader = scene.datasets.read();
    let usage = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    reap_namespace(&backend, &key.namespace);
    offered.prune();
    assert!(dataset.upgrade().is_some());
    drop(reader);
    wait_until(|| {
        offered.prune();
        worker.prune();
        dataset.upgrade().is_none() && generation.upgrade().is_none()
    });
    assert!(offered.candidates().is_empty());
    assert!(extracted.candidates().is_empty());
    assert!(set.datasets.is_empty());
    assert!(backend.budget().usage().allocated < usage.allocated);
    assert!(backend.budget().usage().committed() <= usage.limit);
    drop(pressure);
}

#[test]
fn sleeping_worker_releases_superseded_cache_and_failed_retry_after_retirement() {
    let backend = DrawingBackend::default();
    let worker = Worker::new(backend.clone()).unwrap();
    let (first, key, revision) = published_snapshot(&backend);
    let old = Arc::downgrade(&first.entries.read()[0].generation);
    worker.submit(first.clone());
    let mut finished = None;
    wait_until(|| {
        finished = worker.take();
        finished.is_some()
    });
    drop(finished);
    let next = first.entries.read()[0]
        .generation
        .restyle(backend.budget(), &Presentation::default())
        .unwrap();
    let new = Arc::downgrade(&next);
    {
        let registry = backend.registry();
        let mut registry = registry.lock().unwrap();
        let permit = registry
            .begin_dataset(&key, DatasetKind::Cloud, Some(revision))
            .unwrap();
        registry.commit_dataset(permit, next).unwrap();
    }
    drop(first);
    let mut seen = u64::MAX;
    let next = Snapshot::capture(&backend, &mut seen).unwrap().unwrap();
    let usage = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    worker.submit(next);
    wait_until(|| worker.shared.0.lock().unwrap().pending.is_none());
    assert!(old.upgrade().is_some());
    reap_namespace(&backend, &key.namespace);
    wait_until(|| {
        worker.prune();
        old.upgrade().is_none() && new.upgrade().is_none()
    });
    drop(pressure);
}

#[test]
fn failure_identity_keeps_metadata_but_not_source_generation_payloads() {
    let backend = DrawingBackend::default();
    let (snapshot, key, _) = published_snapshot(&backend);
    let generation = Arc::downgrade(&snapshot.entries.read()[0].generation);
    let failure = {
        let entries = snapshot.entries.read();
        Failure {
            metadata: entries[0].generation.metadata().clone(),
            lifetime: entries[0].lifetime.clone(),
            message: Text::new(&"invalid source"),
        }
    };
    drop(snapshot);
    reap_namespace(&backend, &key.namespace);
    assert!(generation.upgrade().is_none());
    assert_eq!(failure.key(), &key);
    assert_eq!(failure.message.to_string(), "invalid source");
}

#[test]
fn preparation_retries_after_cpu_pressure_without_another_commit() {
    for scene_allowance in [
        0,
        size_of::<Scene>() + 16 + size_of::<Arc<Dataset>>() + size_of::<(DataKey, String)>() + 1024,
    ] {
        let backend = DrawingBackend::default();
        backend.enable_renderer_reports().unwrap();
        let (snapshot, key, revision) = published_snapshot(&backend);
        let worker = Worker::new(backend.clone()).unwrap();
        let usage = backend.budget().usage();
        let pressure = backend
            .budget()
            .reserve(usage.limit - usage.committed() - scene_allowance as u64)
            .unwrap();
        let sequence = worker.submit(snapshot.clone());
        wait_until(|| {
            backend
                .render_failure(&key, revision)
                .is_some_and(|f| f.code == data_code::BUDGET)
        });
        drop(pressure);
        wait_until(|| {
            worker.take().is_some_and(|scene| {
                scene.sequence == sequence
                    && scene.failures.is_empty()
                    && scene.datasets.len() == 1
                    && scene.datasets.read()[0].revision == revision
            })
        });
        assert!(backend.render_failure(&key, revision).is_none());
        assert_eq!(
            backend
                .registry()
                .lock()
                .unwrap()
                .get(&(key.namespace, key.id))
                .unwrap()
                .revision,
            revision
        );
    }
}

#[test]
fn clearing_a_budget_failed_snapshot_cancels_its_retry() {
    let backend = DrawingBackend::default();
    backend.enable_renderer_reports().unwrap();
    let baseline = backend.budget().usage().committed();
    let (snapshot, key, revision) = published_snapshot(&backend);
    let worker = Worker::new(backend.clone()).unwrap();
    let usage = backend.budget().usage();
    let pressure = backend
        .budget()
        .reserve(usage.limit - usage.committed())
        .unwrap();
    worker.submit(snapshot.clone());
    wait_until(|| backend.render_failure(&key, revision).is_some());
    backend.lock().clear(&key.namespace);
    drop(pressure);
    let mut seen = u64::MAX;
    let empty = Snapshot::capture(&backend, &mut seen).unwrap().unwrap();
    let sequence = worker.submit(empty.clone());
    wait_until(|| {
        worker
            .take()
            .is_some_and(|scene| scene.sequence == sequence && scene.datasets.is_empty())
    });
    drop((worker, snapshot, empty));
    loop {
        let retired = backend.registry().lock().unwrap().pop_retired();
        if retired.is_none() {
            break;
        }
        drop(retired);
    }
    assert_eq!(backend.budget().usage().committed(), baseline);
}

struct Copies(Vec<(PageAddress, Vec<u8>)>);

impl gearbox_api::draw_data::PageProvider for Copies {
    fn owned_bytes(&self) -> u64 {
        let table = self.0.capacity() * size_of::<(PageAddress, Vec<u8>)>();
        let bytes: usize = self.0.iter().map(|(_, bytes)| bytes.capacity()).sum();
        (table + bytes) as u64
    }

    fn availability(&self, page: &PageDescriptor) -> gearbox_api::draw_data::ProviderAvailability {
        if self.0.iter().any(|(address, _)| *address == page.address) {
            gearbox_api::draw_data::ProviderAvailability::Available
        } else {
            gearbox_api::draw_data::ProviderAvailability::Missing
        }
    }

    fn load(
        &self,
        sink: &mut gearbox_api::draw_data::PageSink<'_>,
    ) -> Result<(), gearbox_api::draw_data::ProviderError> {
        let address = sink.descriptor().address;
        let (_, bytes) = self
            .0
            .iter()
            .find(|(page, _)| *page == address)
            .expect("provider page");
        for chunk in bytes.chunks(DATA_CHUNK_BYTES) {
            sink.push(chunk)?;
        }
        Ok(())
    }
}

#[test]
fn provider_backed_pages_prepare_after_eviction_and_restyle_without_reloading() {
    use gearbox_api::draw_data::{PagedGeneration, RegisteredProvider};
    let budget = CpuBudget::new(16 << 20);
    let serial = AtomicU64::new(1);
    let points: Vec<[f64; 3]> = (0..6).map(|i| [i as f64, 0.5 * i as f64, 1.0]).collect();
    let entry = retained_paged(&budget, &points, 2);
    let resident = entry.generation.resident().unwrap().clone();
    let copies = Copies(
        resident
            .pages()
            .iter()
            .map(|page| (page.descriptor().address, page.bytes().to_vec()))
            .collect(),
    );
    let provider = RegisteredProvider::new(&budget, copies).unwrap();
    let cancelled = AtomicBool::new(false);
    let request = PageRequest::new(Instant::now() + Duration::from_secs(10), &cancelled);
    let paged = PagedGeneration::verified(&budget, &resident, provider, request).unwrap();
    let Retained {
        revision,
        generation,
        lifetime,
    } = entry;
    drop((resident, generation));
    let entry = Retained {
        revision,
        generation: DatasetStorage::from_paged(&budget, paged.clone()).unwrap(),
        lifetime,
    };
    for page in paged.pages() {
        assert!(page.evict());
    }
    assert_eq!(budget.usage().page_bytes, 0);

    let first = prepare_dataset(&budget, &entry, None, 1, &serial).unwrap();
    assert_eq!(first.pages.len(), 3);
    assert!(
        paged
            .pages()
            .iter()
            .all(|page| page.statistics().loads == 1)
    );
    for (index, page) in first.pages.iter().enumerate() {
        assert_eq!(
            page.appearance
                .geometry
                .source()
                .unwrap()
                .source_ids()
                .unwrap()
                .next(),
            Some(u64::MAX - index as u64 * 2)
        );
    }

    for page in paged.pages() {
        assert!(page.evict());
    }
    let mut presentation = entry.generation.descriptor().presentation.clone();
    presentation.color = ColorMode::Constant {
        rgba: [40, 200, 160, 255],
    };
    let next = Retained {
        revision: DatasetRevision {
            generation: 2,
            ..entry.revision
        },
        generation: entry.generation.restyle(&budget, &presentation).unwrap(),
        lifetime: entry.lifetime.clone(),
    };
    let second = prepare_dataset(&budget, &next, Some(&first), 1, &serial).unwrap();
    for (old, new) in first.pages.iter().zip(&second.pages) {
        assert!(Arc::ptr_eq(
            &old.appearance.geometry,
            &new.appearance.geometry
        ));
    }
    assert!(
        paged
            .pages()
            .iter()
            .all(|page| page.statistics().loads == 1)
    );
    drop((first, second, entry, next, paged));
    assert_eq!(budget.usage().allocated, 0);
}

struct Switched {
    copies: Copies,
    missing: Arc<AtomicU64>,
}

impl gearbox_api::draw_data::PageProvider for Switched {
    fn owned_bytes(&self) -> u64 {
        self.copies.owned_bytes()
    }

    fn availability(&self, page: &PageDescriptor) -> gearbox_api::draw_data::ProviderAvailability {
        if page.address.page_id == self.missing.load(Ordering::Acquire) {
            gearbox_api::draw_data::ProviderAvailability::Missing
        } else {
            self.copies.availability(page)
        }
    }

    fn load(
        &self,
        sink: &mut gearbox_api::draw_data::PageSink<'_>,
    ) -> Result<(), gearbox_api::draw_data::ProviderError> {
        self.copies.load(sink)
    }
}

fn paged_entry(
    budget: &CpuBudget,
) -> (
    Retained,
    Arc<gearbox_api::draw_data::PagedGeneration>,
    Arc<AtomicU64>,
) {
    use gearbox_api::draw_data::{PagedGeneration, RegisteredProvider};
    let points: Vec<[f64; 3]> = (0..6).map(|i| [i as f64, 0.5 * i as f64, 1.0]).collect();
    let Retained {
        revision,
        generation,
        lifetime,
    } = retained_paged(budget, &points, 2);
    let resident = generation.resident().unwrap().clone();
    let missing = Arc::new(AtomicU64::new(u64::MAX));
    let provider = Switched {
        copies: Copies(
            resident
                .pages()
                .iter()
                .map(|page| (page.descriptor().address, page.bytes().to_vec()))
                .collect(),
        ),
        missing: missing.clone(),
    };
    let provider = RegisteredProvider::new(budget, provider).unwrap();
    let cancelled = AtomicBool::new(false);
    let request = PageRequest::new(Instant::now() + Duration::from_secs(10), &cancelled);
    let paged = PagedGeneration::verified(budget, &resident, provider, request).unwrap();
    drop((resident, generation));
    let entry = Retained {
        revision,
        generation: DatasetStorage::from_paged(budget, paged.clone()).unwrap(),
        lifetime,
    };
    (entry, paged, missing)
}

fn demand(budget: &CpuBudget, entry: &Retained, pages: &[u64]) -> Demand {
    Demand::new(
        budget,
        pages.iter().map(|&page| super::super::lod::PageKey {
            incarnation: entry.revision.incarnation,
            scan: 0,
            page,
        }),
    )
    .unwrap()
}

#[test]
fn demand_hollows_other_pages_and_reloads_them_with_their_original_metadata() {
    let budget = CpuBudget::new(16 << 20);
    let serial = AtomicU64::new(1);
    let (entry, paged, _) = paged_entry(&budget);
    let full = prepare_resident(&budget, &entry, None, 1, &serial, None).unwrap();
    assert!(
        full.pages
            .iter()
            .all(|page| page.appearance.geometry.resident())
    );
    assert!(full.settled(None));

    let near = demand(&budget, &entry, &[0]);
    assert!(!full.settled(Some(&near)));
    let partial = prepare_resident(&budget, &entry, Some(&full), 1, &serial, Some(&near)).unwrap();
    assert!(partial.settled(Some(&near)));
    assert_eq!(
        partial.paging,
        Paging {
            pages: 3,
            resident: 1,
            hollow: 2,
            ..Paging::default()
        }
    );
    assert!(Arc::ptr_eq(&full.pages[0], &partial.pages[0]));
    for index in 1..3 {
        let (old, new) = (&full.pages[index], &partial.pages[index]);
        let geometry = &new.appearance.geometry;
        assert!(!geometry.resident() && geometry.source().is_none());
        assert!(geometry.positions.values.is_empty() && new.appearance.attributes.words.values.is_empty());
        assert_eq!(geometry.len(), old.appearance.geometry.len());
        assert!(Arc::ptr_eq(
            &geometry.metadata,
            &old.appearance.geometry.metadata
        ));
        assert!(Arc::ptr_eq(&new.appearance.coarse, &old.appearance.coarse));
        let ordinal = geometry.metadata.coarse.ordinals.values[0];
        assert_eq!(
            geometry.local_point(ordinal),
            old.appearance.geometry.local_point(ordinal)
        );
        assert_ne!(
            paged.pages()[index].status(),
            gearbox_api::draw_data::PageState::Resident
        );
    }
    let page_bytes = |dataset: &Dataset| {
        dataset
            .pages
            .iter()
            .filter_map(|page| page.appearance.geometry.source())
            .map(|source| source.bytes().len() as u64)
            .sum::<u64>()
    };
    let resident_bytes = page_bytes(&partial);
    drop(full);
    assert_eq!(budget.usage().page_bytes, resident_bytes);

    let all = demand(&budget, &entry, &[0, 1, 2]);
    let restored =
        prepare_resident(&budget, &entry, Some(&partial), 1, &serial, Some(&all)).unwrap();
    assert!(restored.settled(Some(&all)));
    assert_eq!(restored.paging.resident, 3);
    assert_eq!(restored.paging.misses, 2);
    for index in 1..3 {
        let (old, new) = (&partial.pages[index], &restored.pages[index]);
        assert!(new.appearance.geometry.resident());
        assert!(Arc::ptr_eq(
            &new.appearance.geometry.metadata,
            &old.appearance.geometry.metadata
        ));
        assert_eq!(
            new.appearance.geometry.positions.values.len(),
            new.appearance.geometry.len()
        );
        assert_eq!(paged.pages()[index].statistics().loads, 1);
    }
    drop((partial, restored, entry, paged, near, all));
    assert_eq!(budget.usage().allocated, 0);
}

#[test]
fn unavailable_reload_keeps_the_coarse_page_and_reports_it() {
    let budget = CpuBudget::new(16 << 20);
    let serial = AtomicU64::new(1);
    let (entry, _paged, missing) = paged_entry(&budget);
    let none = demand(&budget, &entry, &[]);
    let hollow = prepare_resident(&budget, &entry, None, 1, &serial, Some(&none)).unwrap();
    assert_eq!(hollow.paging.hollow, 3);
    missing.store(2, Ordering::Release);
    let all = demand(&budget, &entry, &[0, 1, 2]);
    let partial = prepare_resident(&budget, &entry, Some(&hollow), 1, &serial, Some(&all)).unwrap();
    assert_eq!(partial.paging.unavailable, 1);
    assert_eq!(partial.paging.resident, 2);
    assert!(!partial.settled(Some(&all)));
    assert!(Arc::ptr_eq(&partial.pages[2], &hollow.pages[2]));
    missing.store(u64::MAX, Ordering::Release);
    let recovered =
        prepare_resident(&budget, &entry, Some(&partial), 1, &serial, Some(&all)).unwrap();
    assert!(recovered.settled(Some(&all)));
    assert_eq!(recovered.paging.unavailable, 0);
}

#[test]
fn a_demand_queued_beside_a_new_snapshot_is_answered_by_that_snapshot() {
    let backend = DrawingBackend::default();
    let worker = Worker::new(backend.clone()).unwrap();
    let (snapshot, _, _) = published_snapshot(&backend);
    let latest = {
        let mut mailbox = worker.shared.0.lock().unwrap();
        let submitted = worker.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        mailbox.pending = Some((submitted, snapshot));
        mailbox.repeat = true;
        worker.sequence.fetch_add(1, Ordering::AcqRel) + 1
    };
    worker.shared.1.notify_one();
    let mut finished = None;
    wait_until(|| {
        finished = worker.take();
        finished.is_some()
    });
    assert_eq!(finished.unwrap().sequence, latest, "the runtime discards any other sequence");
}
