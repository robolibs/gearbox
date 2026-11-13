use super::*;
use gearbox_api::draw_data::{DatasetMetadata, Generation, ScanMetadata, StreamingStore};
use gearbox_api::wire::draw_data::*;

fn descriptor(clock: AgeClock) -> DataDescriptor {
    DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new("clock-tests", "cloud").unwrap(),
        shape: DataShape::Cloud {
            schema: super::super::tests::fixture(&[], vec![]).0,
        },
        stream: StreamMode::Accumulate {
            age_clock: clock,
            max_scans: MAX_SCANS,
            max_age_s: 1e12,
            max_bytes: 1 << 20,
        },
        lifetime_s: None,
        presentation: Presentation::default(),
    }
}

#[test]
fn only_simulation_age_changes_page_parameters_with_simulation_time() {
    use super::super::age::AgeUniform;
    let scan = scan(1, Some("source"), u64::MAX);
    let capture = scan.frame.capture.as_ref();
    let simulated = ColorMode::Age {
        clock: AgeClock::Simulation,
        colormap: gearbox_api::colormap::GREY,
        range: [0.0, 5.0],
    };
    let source = ColorMode::Age {
        clock: AgeClock::Source,
        colormap: gearbox_api::colormap::GREY,
        range: [0.0, 5.0],
    };
    assert_ne!(
        AgeUniform::new(&simulated, capture, 1.5, u64::MAX),
        AgeUniform::new(&simulated, capture, 0.5, u64::MAX)
    );
    for now in [0.0, 1.5, 100.0, f64::MAX] {
        assert_eq!(
            AgeUniform::new(&source, capture, now, u64::MAX),
            AgeUniform::new(&source, capture, 0.0, u64::MAX)
        );
        assert_eq!(
            AgeUniform::new(&Presentation::default().color, capture, now, u64::MAX),
            AgeUniform::default()
        );
    }
}

fn scan(key: u64, clock: Option<&str>, source_ns: u64) -> ScanDescriptor {
    let stamp = CaptureStamp {
        simulation_time_s: key as f64,
        source_time_ns: clock.map(|_| source_ns),
        source_clock: clock.map(str::to_owned),
        sample: key,
        scene_epoch: 1,
    };
    ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: key,
        source: None,
        frame: DataFrame {
            placement: Placement::Snapshot,
            capture: Some(stamp.clone()),
            capture_transform: Some(CaptureTransform {
                revision: SCHEMA_REVISION,
                source_frame: "world".into(),
                target_frame: "world".into(),
                stamp,
                pose: RigidPose::default(),
                datum: Some(DatumSnapshot {
                    origin_ecef_m: [6_378_137.0, 0.0, 0.0],
                    axes_to_ecef_wxyz: [1.0, 0.0, 0.0, 0.0],
                }),
            }),
            ..Default::default()
        },
        source_count: 0,
        organization: None,
    }
}

fn generation(
    budget: &CpuBudget,
    clock: AgeClock,
    scans: &[ScanDescriptor],
) -> Arc<DatasetStorage> {
    let metadata = DatasetMetadata::new(budget, &descriptor(clock)).unwrap();
    let scans: Vec<_> = scans
        .iter()
        .map(|scan| ScanMetadata::new(budget, scan).unwrap())
        .collect();
    DatasetStorage::from_resident(
        budget,
        Generation::build(budget, metadata, &scans, &[]).unwrap(),
    )
    .unwrap()
}

#[test]
fn named_source_clocks_are_independent_and_preserve_all_timestamp_bits() {
    let budget = CpuBudget::new(16 << 20);
    let scans = [
        scan(1, Some("lidar"), 7),
        scan(2, Some("gps"), u64::MAX - 1),
        scan(3, Some("lidar"), 9),
        scan(4, Some("gps"), u64::MAX),
        scan(5, Some("Lidar"), 123),
        scan(6, None, 0),
    ];
    let generation = generation(&budget, AgeClock::Simulation, &scans);
    let clocks = SourceClocks::prepare(&budget, generation).unwrap();
    assert_eq!(clocks.indices.len(), 3);
    for (scan, expected) in scans.iter().zip([
        Some(9),
        Some(u64::MAX),
        Some(9),
        Some(u64::MAX),
        Some(123),
        None,
    ]) {
        assert_eq!(clocks.time_ns(scan.frame.capture.as_ref()), expected);
    }
    assert_eq!(clocks.time_ns(None), None);
    let mut missing = scans[0].frame.capture.clone().unwrap();
    missing.scene_epoch = 2;
    assert_eq!(clocks.time_ns(Some(&missing)), None);
    missing.scene_epoch = 1;
    missing.source_clock = Some("unknown".into());
    assert_eq!(clocks.time_ns(Some(&missing)), None);
    drop(clocks);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn duplicate_clocks_have_charged_bounded_capacity_and_budget_failure_releases_it() {
    let budget = CpuBudget::new(16 << 20);
    let scans: Vec<_> = (1..=MAX_SCANS)
        .map(|key| scan(key as u64, Some("one"), key as u64))
        .collect();
    let generation = generation(&budget, AgeClock::Simulation, &scans);
    let baseline = budget.usage().committed();
    let pressure = budget.reserve(budget.usage().limit - baseline - 1).unwrap();
    assert!(matches!(
        SourceClocks::prepare(&budget, generation.clone()),
        Err(StorageError::Budget { .. })
    ));
    drop(pressure);
    assert_eq!(budget.usage().committed(), baseline);
    let clocks = SourceClocks::prepare(&budget, generation.clone()).unwrap();
    assert_eq!(clocks.indices.len(), 1);
    assert_eq!(clocks.indices.capacity(), scans.len());
    assert_eq!(
        budget.usage().committed() - baseline,
        (scans.len() * size_of::<usize>()) as u64
    );
    assert_eq!(
        clocks.time_ns(scans[0].frame.capture.as_ref()),
        Some(MAX_SCANS as u64)
    );
    drop((clocks, generation));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn source_clock_lookup_is_unchanged_by_palette_and_blur_updates() {
    let budget = CpuBudget::new(16 << 20);
    let scans = [scan(1, Some("a"), 42), scan(2, Some("b"), u64::MAX)];
    let generation = generation(&budget, AgeClock::Simulation, &scans);
    let presentation = Presentation {
        color: ColorMode::Age {
            clock: AgeClock::Source,
            colormap: gearbox_api::colormap::OKABE,
            range: [0.0, 10.0],
        },
        blur: 0.7,
        ..Default::default()
    };
    let restyled = generation.restyle(&budget, &presentation).unwrap();
    assert!(Arc::ptr_eq(&generation.scans()[0], &restyled.scans()[0]));
    let before = SourceClocks::prepare(&budget, generation).unwrap();
    let after = SourceClocks::prepare(&budget, restyled).unwrap();
    for scan in scans {
        assert_eq!(
            before.time_ns(scan.frame.capture.as_ref()),
            after.time_ns(scan.frame.capture.as_ref())
        );
    }
    drop((before, after));
    assert_eq!(budget.usage().committed(), 0);
}

struct Store {
    inner: StreamingStore,
    lease: u64,
    next: u64,
}

impl Store {
    fn new() -> Self {
        let mut inner = StreamingStore::new(1, DataLimits::reference()).unwrap();
        let reply = inner.handle(
            &DataRequest {
                host_epoch: 1,
                ..DataRequest::new(data_op::OPEN_WRITER)
            },
            0,
        );
        assert_eq!(reply.code, data_code::OK);
        Self {
            inner,
            lease: reply.lease,
            next: 1,
        }
    }

    fn replace(&mut self, clock: AgeClock, scans: &[ScanDescriptor]) -> Arc<DatasetStorage> {
        let descriptor = descriptor(clock);
        let base = self
            .inner
            .registry()
            .lock()
            .unwrap()
            .get(&(descriptor.key.namespace.clone(), descriptor.key.id.clone()))
            .map(|entry| entry.revision);
        let mut request = DataRequest {
            host_epoch: 1,
            lease: self.lease,
            transaction: self.next,
            incarnation: base.map_or(0, |base| base.incarnation),
            base_generation: base.map_or(0, |base| base.generation),
            ..DataRequest::new(data_op::BEGIN)
        }
        .with_key(&descriptor.key)
        .unwrap()
        .with_metadata(&BeginMetadata {
            descriptor: descriptor.clone(),
            mode: CommitMode::Replace,
            scans: scans.len() as u32,
            pages: 0,
            removed_scans: 0,
            removed_pages: 0,
        })
        .unwrap();
        self.call(&request);
        request.operation = data_op::DECLARE_SCAN;
        for scan in scans {
            self.call(
                &DataRequest {
                    scan_key: scan.scan_key,
                    ..request.clone()
                }
                .with_metadata(scan)
                .unwrap(),
            );
        }
        request.operation = data_op::COMMIT;
        request.metadata.clear();
        self.call(&request);
        self.next += 1;
        self.inner
            .registry()
            .lock()
            .unwrap()
            .get(&(descriptor.key.namespace, descriptor.key.id))
            .unwrap()
            .payload
            .clone()
    }

    fn call(&mut self, request: &DataRequest) {
        let reply = self.inner.handle(request, 0);
        assert_eq!(
            reply.code,
            data_code::OK,
            "{}",
            String::from_utf8_lossy(&reply.message)
        );
    }
}

#[test]
fn source_policy_keeps_its_committed_anchor_after_empty_replace_and_late_capture() {
    let mut store = Store::new();
    let first = scan(1, Some("sensor"), u64::MAX);
    store.replace(AgeClock::Source, &[first.clone()]);
    let empty = store.replace(AgeClock::Source, &[]);
    let clocks = SourceClocks::prepare(store.inner.budget(), empty).unwrap();
    assert_eq!(clocks.time_ns(first.frame.capture.as_ref()), Some(u64::MAX));
    let late = scan(2, Some("sensor"), u64::MAX - 1);
    let later = store.replace(AgeClock::Source, &[late.clone()]);
    let later = SourceClocks::prepare(store.inner.budget(), later).unwrap();
    assert_eq!(later.time_ns(late.frame.capture.as_ref()), Some(u64::MAX));
}

#[test]
fn simulation_policy_does_not_reuse_an_evicted_simulation_anchor_as_a_source_clock() {
    let mut store = Store::new();
    let first = scan(1, Some("sensor"), u64::MAX);
    store.replace(AgeClock::Simulation, &[first.clone()]);
    let empty = store.replace(AgeClock::Simulation, &[]);
    assert!(empty.accumulation_clock_anchor().is_some());
    let clocks = SourceClocks::prepare(store.inner.budget(), empty).unwrap();
    assert_eq!(clocks.time_ns(first.frame.capture.as_ref()), None);
    let later = scan(2, Some("sensor"), 10);
    let later_generation = store.replace(AgeClock::Simulation, &[later.clone()]);
    let later_clocks = SourceClocks::prepare(store.inner.budget(), later_generation).unwrap();
    assert_eq!(later_clocks.time_ns(later.frame.capture.as_ref()), Some(10));
}

#[test]
fn accumulation_policy_change_resets_source_reference_without_renderer_history() {
    let mut store = Store::new();
    store.replace(AgeClock::Source, &[scan(1, Some("old"), u64::MAX)]);
    let next = scan(2, Some("new"), 15);
    let generation = store.replace(AgeClock::Simulation, &[next.clone()]);
    let clocks = SourceClocks::prepare(store.inner.budget(), generation).unwrap();
    assert_eq!(clocks.time_ns(next.frame.capture.as_ref()), Some(15));
    assert_eq!(
        clocks.time_ns(scan(1, Some("old"), u64::MAX).frame.capture.as_ref()),
        None
    );
}
