use std::sync::Arc;

use gearbox_api::draw_data::*;
use gearbox_api::wire::draw_data::*;

#[allow(dead_code)]
#[path = "support/draw_data_contracts.rs"]
mod contracts;

fn schema() -> CloudSchema {
    CloudSchema {
        revision: 1,
        attributes: vec![
            Attribute {
                name: "xyz".into(),
                scalar: ScalarType::F32,
                components: 3,
                semantic: AttributeSemantic::Position,
                units: "m".into(),
                normalization: Normalization::None,
                nullable: true,
            },
            Attribute {
                name: "id".into(),
                scalar: ScalarType::U64,
                components: 1,
                semantic: AttributeSemantic::SourceId,
                units: "".into(),
                normalization: Normalization::None,
                nullable: false,
            },
        ],
    }
}

fn descriptor() -> DataDescriptor {
    DataDescriptor {
        shape: DataShape::Cloud { schema: schema() },
        ..contracts::cloud()
    }
}

fn scan(key: u64, count: u64) -> ScanDescriptor {
    ScanDescriptor {
        source_count: count,
        organization: None,
        ..contracts::scan(key)
    }
}

fn page(
    budget: &CpuBudget,
    scan_key: u64,
    page_id: u64,
    ids: &[u64],
    invalid: bool,
) -> Arc<StoredPage> {
    let count = ids.len() as u32;
    let input = CloudPage {
        count,
        columns: vec![
            ColumnData {
                values: vec![0; ids.len() * 12],
                validity: if invalid {
                    vec![0; ids.len().div_ceil(8)]
                } else {
                    vec![]
                },
            },
            ColumnData {
                values: ids.iter().flat_map(|id| id.to_le_bytes()).collect(),
                validity: vec![],
            },
        ],
    };
    let bytes = input.encode(&schema()).unwrap();
    let descriptor = PageDescriptor {
        address: PageAddress { scan_key, page_id },
        encoding: PageEncoding::CloudColumnsV1,
        items: count,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let mut upload = PageUpload::new(budget, descriptor).unwrap();
    for (index, chunk) in bytes.chunks(DATA_CHUNK_BYTES).enumerate() {
        upload
            .push((index * DATA_CHUNK_BYTES) as u64, chunk)
            .unwrap();
    }
    upload.finish_cloud(&schema()).unwrap().into_page()
}

fn occupancy(budget: &CpuBudget, page_id: u64, key: VoxelKey) -> Arc<StoredPage> {
    let DataShape::Octree { config, .. } = contracts::octree().shape else {
        unreachable!()
    };
    let bytes = encode_occupancy_page(
        &config,
        &[OccupancyLeaf {
            key,
            value: OccupancyValue {
                probability: None,
                confidence: None,
                class_id: None,
            },
        }],
    )
    .unwrap();
    let descriptor = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id,
        },
        encoding: PageEncoding::OccupancyLeavesV1,
        items: 1,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let mut upload = PageUpload::new(budget, descriptor).unwrap();
    upload.push(0, &bytes).unwrap();
    upload.finish_occupancy(&config).unwrap().into_page()
}

fn zero(budget: &CpuBudget) {
    assert_eq!(budget.usage().committed(), 0, "{:?}", budget.usage());
}

#[test]
fn split_reservations_transfer_capacity_without_double_accounting() {
    let budget = CpuBudget::new(100);
    let mut transaction = budget.reserve(100).unwrap();
    let mut page = transaction.split(60).unwrap();
    let before = budget.usage();
    assert_eq!(transaction.bytes(), 40);
    assert!(transaction.split(41).is_err());
    assert_eq!(budget.usage(), before);
    let charge = page.charge(60, MemoryClass::Page).unwrap();
    assert_eq!(budget.usage().committed(), 100);
    drop(transaction);
    drop(page);
    assert_eq!(budget.usage().committed(), 60);
    drop(charge);
    zero(&budget);
}

#[test]
fn page_upload_consumes_a_transaction_reservation_without_losing_other_capacity() {
    let budget = CpuBudget::new(1 << 20);
    let bytes = CloudPage {
        count: 0,
        columns: vec![
            ColumnData {
                values: vec![],
                validity: vec![],
            },
            ColumnData {
                values: vec![],
                validity: vec![],
            },
        ],
    }
    .encode(&schema())
    .unwrap();
    let descriptor = PageDescriptor {
        address: PageAddress {
            scan_key: 1,
            page_id: 1,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: 0,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let size = PageFootprint::for_descriptor(&descriptor).unwrap().total();
    let mut transaction = budget.reserve(size + 100).unwrap();
    let mut pending =
        PageUpload::from_reservation(descriptor.clone(), transaction.split(size).unwrap()).unwrap();
    assert_eq!(budget.usage().committed(), size + 100);
    pending.push(0, &bytes).unwrap();
    let page = pending.finish_cloud(&schema()).unwrap().into_page();
    assert_eq!(budget.usage().reserved, 100);
    drop(page);
    assert_eq!(budget.usage().committed(), 100);
    assert!(PageUpload::from_reservation(descriptor, transaction.split(10).unwrap()).is_err());
    assert_eq!(transaction.bytes(), 90);
    drop(transaction);
    zero(&budget);
}

#[test]
fn scan_identity_allows_repeated_source_ids_across_distinct_capture_poses() {
    let budget = CpuBudget::new(1 << 20);
    let metadata = DatasetMetadata::new(&budget, &descriptor()).unwrap();
    let a = ScanMetadata::new(&budget, &scan(1, 2)).unwrap();
    let b = ScanMetadata::new(&budget, &scan(2, 2)).unwrap();
    let p1 = page(&budget, 1, 8, &[0, u64::MAX], false);
    let p2 = page(&budget, 2, 3, &[0, u64::MAX], true);
    let generation = Generation::build(&budget, metadata, &[b, a], &[p2, p1]).unwrap();
    assert_eq!(
        generation.counts(),
        GenerationCounts {
            scans: 2,
            pages: 2,
            source_samples: 4,
            valid_samples: 2,
            payload_bytes: generation
                .pages()
                .iter()
                .map(|p| p.descriptor().byte_len)
                .sum(),
        }
    );
    assert_eq!(
        generation.scans()[0]
            .value()
            .frame
            .capture_transform
            .as_ref()
            .unwrap()
            .pose
            .translation_m,
        [10.0, 0.0, 0.0]
    );
    assert_eq!(
        generation.scans()[1]
            .value()
            .frame
            .capture_transform
            .as_ref()
            .unwrap()
            .pose
            .translation_m,
        [20.0, 0.0, 0.0]
    );
    assert_eq!(
        generation
            .page(PageAddress {
                scan_key: 2,
                page_id: 3
            })
            .unwrap()
            .source_ids()
            .unwrap()
            .collect::<Vec<_>>(),
        [0, u64::MAX]
    );
    assert!(
        generation
            .page(PageAddress {
                scan_key: 2,
                page_id: 8
            })
            .is_none()
    );
    assert_eq!(budget.usage().scratch_bytes, 0);
    assert_eq!(budget.usage().reserved, 0);
    drop(generation);
    zero(&budget);
}

#[test]
fn cross_page_duplicate_ids_include_invalid_observations() {
    let budget = CpuBudget::new(1 << 20);
    let metadata = DatasetMetadata::new(&budget, &descriptor()).unwrap();
    let scan = ScanMetadata::new(&budget, &scan(1, 2)).unwrap();
    let pages = [
        page(&budget, 1, 0, &[u64::MAX], false),
        page(&budget, 1, 1, &[u64::MAX], true),
    ];
    let baseline = budget.usage().allocated;
    assert!(Generation::build(&budget, metadata.clone(), &[scan.clone()], &pages).is_err());
    assert_eq!(budget.usage().allocated, baseline);
    assert_eq!(budget.usage().reserved, 0);
    drop((metadata, scan, pages));
    zero(&budget);
}

#[test]
fn duplicate_addresses_missing_scans_and_wrong_source_counts_are_rejected() {
    let budget = CpuBudget::new(1 << 20);
    let metadata = DatasetMetadata::new(&budget, &descriptor()).unwrap();
    let scan = ScanMetadata::new(&budget, &scan(1, 2)).unwrap();
    let pages = [
        page(&budget, 1, 0, &[0], false),
        page(&budget, 1, 0, &[1], false),
    ];
    let baseline = budget.usage().allocated;
    assert!(Generation::build(&budget, metadata.clone(), &[scan.clone()], &pages).is_err());
    assert!(Generation::build(&budget, metadata.clone(), &[], &pages[..1]).is_err());
    assert!(Generation::build(&budget, metadata.clone(), &[scan.clone()], &pages[..1]).is_err());
    assert!(
        Generation::build(
            &budget,
            metadata.clone(),
            &[scan.clone(), scan.clone()],
            &pages
        )
        .is_err()
    );
    assert_eq!(budget.usage().allocated, baseline);
    drop((metadata, scan, pages));
    zero(&budget);
}

#[test]
fn generations_reject_cross_budget_borrowing_and_wrong_page_kinds() {
    let budget = CpuBudget::new(1 << 20);
    let other = CpuBudget::new(1 << 20);
    let metadata = DatasetMetadata::new(&budget, &descriptor()).unwrap();
    let scan = ScanMetadata::new(&budget, &scan(1, 1)).unwrap();
    let foreign = page(&other, 1, 1, &[0], false);
    assert!(
        Generation::build(
            &budget,
            metadata.clone(),
            &[scan.clone()],
            &[foreign.clone()]
        )
        .is_err()
    );
    assert!(Generation::build(&other, metadata.clone(), &[], &[]).is_err());
    let octree = DatasetMetadata::new(&budget, &contracts::octree()).unwrap();
    let cloud = page(&budget, 0, 0, &[0], false);
    assert!(Generation::build(&budget, octree.clone(), &[], &[cloud.clone()]).is_err());
    assert!(Generation::build(&budget, octree.clone(), &[scan.clone()], &[]).is_err());
    drop((metadata, scan, foreign, octree, cloud));
    zero(&budget);
    zero(&other);
}

#[test]
fn cross_page_octree_topology_is_atomic_and_keeps_signed_tiles() {
    let budget = CpuBudget::new(1 << 20);
    let metadata = DatasetMetadata::new(&budget, &contracts::octree()).unwrap();
    let root = occupancy(
        &budget,
        0,
        VoxelKey {
            tile: [-1, 0, 0],
            depth: 0,
            morton: 0,
        },
    );
    let child = occupancy(
        &budget,
        1,
        VoxelKey {
            tile: [-1, 0, 0],
            depth: 12,
            morton: 7,
        },
    );
    assert!(
        Generation::build(
            &budget,
            metadata.clone(),
            &[],
            &[child.clone(), root.clone()]
        )
        .is_err()
    );
    let duplicate = occupancy(
        &budget,
        2,
        *root
            .voxel_keys()
            .unwrap()
            .collect::<Vec<_>>()
            .first()
            .unwrap(),
    );
    assert!(
        Generation::build(
            &budget,
            metadata.clone(),
            &[],
            &[root.clone(), duplicate.clone()]
        )
        .is_err()
    );
    let neighbor = occupancy(
        &budget,
        3,
        VoxelKey {
            tile: [0; 3],
            depth: 0,
            morton: 0,
        },
    );
    let generation = Generation::build(
        &budget,
        metadata.clone(),
        &[],
        &[neighbor.clone(), root.clone()],
    )
    .unwrap();
    assert_eq!(generation.counts().source_samples, 2);
    drop((metadata, root, child, duplicate, neighbor, generation));
    zero(&budget);
}

#[test]
fn accumulated_metadata_requires_one_epoch_and_clock_and_fits_scan_and_byte_limits() {
    let budget = CpuBudget::new(1 << 20);
    let mut desc = descriptor();
    desc.stream = StreamMode::Accumulate {
        max_scans: 2,
        max_age_s: 10.0,
        max_bytes: 4096,
        age_clock: AgeClock::Source,
    };
    let metadata = DatasetMetadata::new(&budget, &desc).unwrap();
    let first = ScanMetadata::new(&budget, &scan(1, 1)).unwrap();
    let pages = [
        page(&budget, 1, 0, &[0], false),
        page(&budget, 2, 0, &[0], false),
    ];
    for epoch in [false, true] {
        let mut other = scan(2, 1);
        if epoch {
            other.frame.capture.as_mut().unwrap().scene_epoch = 2;
        } else {
            other.frame.capture.as_mut().unwrap().source_clock = Some("other".into());
        }
        other.frame.capture_transform.as_mut().unwrap().stamp =
            other.frame.capture.clone().unwrap();
        let second = ScanMetadata::new(&budget, &other).unwrap();
        assert!(
            Generation::build(&budget, metadata.clone(), &[first.clone(), second], &pages).is_err()
        );
    }
    let second = ScanMetadata::new(&budget, &scan(2, 1)).unwrap();
    for (max_scans, max_bytes) in [(1, 4096), (2, 1)] {
        desc.stream = StreamMode::Accumulate {
            max_scans,
            max_age_s: 10.0,
            max_bytes,
            age_clock: AgeClock::Source,
        };
        let limited = DatasetMetadata::new(&budget, &desc).unwrap();
        assert!(
            Generation::build(&budget, limited, &[first.clone(), second.clone()], &pages).is_err()
        );
    }
    drop((metadata, first, second, pages));
    zero(&budget);
}

#[test]
fn page_columns_are_checked_against_the_generation_schema() {
    let budget = CpuBudget::new(1 << 20);
    let mut desc = descriptor();
    let DataShape::Cloud { schema } = &mut desc.shape else {
        unreachable!()
    };
    schema.attributes.swap(0, 1);
    let metadata = DatasetMetadata::new(&budget, &desc).unwrap();
    let scan = ScanMetadata::new(&budget, &scan(1, 1)).unwrap();
    let page = page(&budget, 1, 0, &[0], false);
    assert!(Generation::build(&budget, metadata, &[scan], &[page]).is_err());
    zero(&budget);
}

#[test]
fn metadata_is_immutable_and_does_not_retain_unused_input_capacity() {
    let budget = CpuBudget::new(1 << 20);
    let mut desc = descriptor();
    desc.key.id.reserve(1 << 20);
    desc.presentation.color = ColorMode::Constant {
        rgba: [234, 76, 12, 99],
    };
    desc.presentation.blur = 0.72;
    let saved = DatasetMetadata::new(&budget, &desc).unwrap();
    assert!(budget.usage().metadata_bytes < 16 * 1024);
    desc.presentation.blur = 0.0;
    assert_eq!(saved.value().presentation.blur, 0.72);
    assert_eq!(
        saved.value().presentation.color,
        ColorMode::Constant {
            rgba: [234, 76, 12, 99]
        }
    );
    drop(saved);
    zero(&budget);
}

#[test]
fn shared_pages_survive_manifest_replacement_and_clear_until_reader_release() {
    let budget = CpuBudget::new(1 << 20);
    let metadata = DatasetMetadata::new(&budget, &descriptor()).unwrap();
    let scan = ScanMetadata::new(&budget, &scan(1, 1)).unwrap();
    let page = page(&budget, 1, 0, &[u64::MAX], false);
    let first =
        Generation::build(&budget, metadata.clone(), &[scan.clone()], &[page.clone()]).unwrap();
    let base_usage = budget.usage();
    let second =
        Generation::build(&budget, metadata.clone(), &[scan.clone()], &[page.clone()]).unwrap();
    assert_eq!(budget.usage().page_bytes, base_usage.page_bytes);
    assert!(Arc::ptr_eq(&first.pages()[0], &second.pages()[0]));
    let key = metadata.value().key.clone();
    let mut store = DrawingStore::new(4, 4).unwrap();
    let permit = store.begin_dataset(&key, DatasetKind::Cloud, None).unwrap();
    let revision = store.commit_dataset(permit, first.clone()).unwrap();
    let permit = store
        .begin_dataset(&key, DatasetKind::Cloud, Some(revision))
        .unwrap();
    store.commit_dataset(permit, second).unwrap();
    store.clear("").unwrap();
    drop((metadata, scan, page, store));
    assert_eq!(
        first.pages()[0].source_ids().unwrap().next(),
        Some(u64::MAX)
    );
    assert_eq!(budget.usage().page_bytes, base_usage.page_bytes);
    drop(first);
    zero(&budget);
}

#[test]
fn generation_budget_rejection_leaves_existing_manifest_unchanged() {
    let budget = CpuBudget::new(1 << 20);
    let metadata = DatasetMetadata::new(&budget, &descriptor()).unwrap();
    let scan = ScanMetadata::new(&budget, &scan(1, 1)).unwrap();
    let page = page(&budget, 1, 0, &[42], false);
    let previous =
        Generation::build(&budget, metadata.clone(), &[scan.clone()], &[page.clone()]).unwrap();
    let usage = budget.usage();
    let hold = budget.reserve(usage.limit - usage.committed()).unwrap();
    assert!(matches!(
        Generation::build(&budget, metadata, &[scan], &[page]),
        Err(StorageError::Budget { .. })
    ));
    assert_eq!(previous.pages()[0].source_ids().unwrap().next(), Some(42));
    drop(hold);
    drop(previous);
    zero(&budget);
}

#[test]
fn million_point_manifest_fits_one_budget_and_shares_payloads_between_generations() {
    let budget = CpuBudget::new(64 << 20);
    let mut desc = descriptor();
    desc.stream = StreamMode::Latest;
    let metadata = DatasetMetadata::new(&budget, &desc).unwrap();
    let total = u64::from(MAX_PAGE_POINTS) * 16;
    let scan = ScanMetadata::new(&budget, &scan(1, total)).unwrap();
    let pages: Vec<_> = (0..16)
        .map(|index| {
            let first = index * u64::from(MAX_PAGE_POINTS);
            let ids: Vec<_> = (first..first + u64::from(MAX_PAGE_POINTS)).collect();
            page(&budget, 1, index, &ids, false)
        })
        .collect();
    let first = Generation::build(&budget, metadata.clone(), &[scan.clone()], &pages).unwrap();
    let payload_bytes = budget.usage().page_bytes;
    let second = Generation::build(&budget, metadata.clone(), &[scan.clone()], &pages).unwrap();
    assert_eq!(first.counts().source_samples, 1_048_576);
    assert_eq!(second.counts().valid_samples, total);
    assert_eq!(budget.usage().page_bytes, payload_bytes);
    assert!(budget.usage().peak <= 64 << 20);
    assert!(Arc::ptr_eq(&first.pages()[15], &second.pages()[15]));
    assert_eq!(
        second.pages()[15].source_ids().unwrap().next(),
        Some(15 * u64::from(MAX_PAGE_POINTS))
    );
    drop((metadata, scan, pages, first));
    assert_eq!(budget.usage().page_bytes, payload_bytes);
    drop(second);
    zero(&budget);
}

#[test]
fn empty_generations_have_explicit_zero_counts_without_invented_scans() {
    let budget = CpuBudget::new(1 << 20);
    for descriptor in [descriptor(), contracts::octree()] {
        let metadata = DatasetMetadata::new(&budget, &descriptor).unwrap();
        let generation = Generation::build(&budget, metadata, &[], &[]).unwrap();
        assert_eq!(generation.counts(), GenerationCounts::default());
        drop(generation);
        zero(&budget);
    }
}
