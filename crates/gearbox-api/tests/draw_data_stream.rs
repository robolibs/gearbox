use std::sync::{Arc, Barrier};

use gearbox_api::draw_data::*;
use gearbox_api::wire::draw_data::*;

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

fn cloud() -> (PageDescriptor, Vec<u8>) {
    let page = CloudPage {
        count: 2,
        columns: vec![
            ColumnData {
                values: [1.0f32, 2.0, 3.0, 0.0, 0.0, 0.0]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect(),
                validity: vec![1],
            },
            ColumnData {
                values: [0u64, u64::MAX]
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect(),
                validity: vec![],
            },
        ],
    };
    let bytes = page.encode(&schema()).unwrap();
    (descriptor(&bytes, 2, PageEncoding::CloudColumnsV1), bytes)
}

fn descriptor(bytes: &[u8], items: u32, encoding: PageEncoding) -> PageDescriptor {
    PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 1,
        },
        encoding,
        items,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(bytes),
        origin_m: [0.0; 3],
    }
}

fn config() -> OctreeConfig {
    OctreeConfig {
        leaf_resolution_m: 0.125,
        max_depth: 21,
        free_max: 0.3,
        occupied_min: 0.7,
    }
}

fn leaf(tile: [i64; 3], depth: u8, morton: u64) -> OccupancyLeaf {
    OccupancyLeaf {
        key: VoxelKey {
            tile,
            depth,
            morton,
        },
        value: OccupancyValue {
            probability: None,
            confidence: Some(0.5),
            class_id: Some(42),
        },
    }
}

fn upload(budget: &CpuBudget, descriptor: PageDescriptor, bytes: &[u8]) -> PageUpload {
    let mut upload = PageUpload::new(budget, descriptor).unwrap();
    for (i, chunk) in bytes.chunks(DATA_CHUNK_BYTES).enumerate() {
        assert_eq!(
            upload.push((i * DATA_CHUNK_BYTES) as u64, chunk).unwrap(),
            ChunkOutcome::Accepted
        );
    }
    upload
}

fn zero(budget: &CpuBudget) {
    let usage = budget.usage();
    assert_eq!(usage.committed(), 0, "{usage:?}");
    assert_eq!(
        usage.page_bytes + usage.upload_index_bytes + usage.scratch_bytes + usage.metadata_bytes,
        0
    );
    assert!(usage.peak <= usage.limit);
}

#[test]
fn reservation_conversion_does_not_double_charge_and_drop_releases_capacity() {
    let budget = CpuBudget::new(100);
    let mut reserve = budget.reserve(100).unwrap();
    assert!(budget.reserve(1).is_err());
    let charge = reserve.charge(60, MemoryClass::Page).unwrap();
    assert_eq!(charge.bytes(), 60);
    assert_eq!(reserve.bytes(), 40);
    let before = budget.usage();
    assert_eq!(
        (before.reserved, before.allocated, before.page_bytes),
        (40, 60, 60)
    );
    assert!(reserve.charge(41, MemoryClass::Scratch).is_err());
    assert_eq!(budget.usage(), before);
    drop(reserve);
    assert_eq!(budget.usage().committed(), 60);
    drop(charge);
    zero(&budget);
}

#[test]
fn budget_limits_handle_zero_and_u64_max_without_overflow() {
    assert!(CpuBudget::new(0).reserve(1).is_err());
    let budget = CpuBudget::new(u64::MAX);
    let mut reservation = budget.reserve(u64::MAX).unwrap();
    let charge = reservation.charge(u64::MAX, MemoryClass::Metadata).unwrap();
    assert!(budget.reserve(1).is_err());
    assert_eq!(budget.usage().peak, u64::MAX);
    drop(charge);
    drop(reservation);
    zero(&budget);
}

#[test]
fn concurrent_reservations_obey_one_global_cap() {
    let budget = CpuBudget::new(8 * 32);
    let barrier = Arc::new(Barrier::new(16));
    let handles: Vec<_> = (0..16)
        .map(|_| {
            let budget = budget.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let reserved = budget.reserve(32);
                barrier.wait();
                assert_eq!(budget.usage().committed(), 256);
                barrier.wait();
                reserved.is_ok()
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum::<usize>(),
        8
    );
    zero(&budget);
}

#[test]
fn page_declaration_reserves_payload_bitmaps_metadata_and_peak_scratch() {
    let (descriptor, _) = cloud();
    let footprint = PageFootprint::for_descriptor(&descriptor).unwrap();
    let small = CpuBudget::new(footprint.total() - 1);
    assert!(matches!(
        PageUpload::new(&small, descriptor.clone()),
        Err(StorageError::Budget { .. })
    ));
    zero(&small);
    let budget = CpuBudget::new(footprint.total());
    let upload = PageUpload::new(&budget, descriptor).unwrap();
    let usage = budget.usage();
    assert_eq!(usage.committed(), footprint.total());
    assert_eq!(usage.reserved, footprint.scratch);
    assert_eq!(usage.page_bytes, footprint.payload);
    assert_eq!(usage.upload_index_bytes, footprint.upload_index);
    assert_eq!(usage.metadata_bytes, footprint.metadata);
    assert!(!upload.is_complete());
    drop(upload);
    zero(&budget);
}

#[test]
fn malformed_page_declarations_allocate_nothing() {
    let budget = CpuBudget::new(1 << 20);
    let (descriptor, _) = cloud();
    for bad in [
        PageDescriptor {
            byte_len: u64::MAX,
            ..descriptor.clone()
        },
        PageDescriptor {
            items: MAX_PAGE_POINTS + 1,
            ..descriptor.clone()
        },
        PageDescriptor {
            origin_m: [f64::NAN; 3],
            ..descriptor.clone()
        },
    ] {
        assert!(PageUpload::new(&budget, bad).is_err());
        zero(&budget);
        assert_eq!(budget.usage().peak, 0);
    }
}

#[test]
fn retries_require_exact_original_boundaries_in_every_arrival_order() {
    let bytes = [0u8; 32];
    let ranges = [0..7, 7..19, 19..32];
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let budget = CpuBudget::new(1 << 20);
        let mut upload =
            PageUpload::new(&budget, descriptor(&bytes, 0, PageEncoding::CloudColumnsV1)).unwrap();
        let mut accepted = Vec::new();
        for part in order {
            let range = ranges[part].clone();
            assert_eq!(
                upload
                    .push(range.start as u64, &bytes[range.clone()])
                    .unwrap(),
                ChunkOutcome::Accepted
            );
            accepted.push(range.clone());
            for start in range.clone() {
                for end in start + 1..=range.end {
                    let expected = if accepted.contains(&(start..end)) {
                        Ok(ChunkOutcome::Duplicate)
                    } else {
                        Err(StorageError::Conflict)
                    };
                    assert_eq!(
                        upload.push(start as u64, &bytes[start..end]),
                        expected,
                        "{order:?} {start}..{end}"
                    );
                }
            }
        }
        for start in 0..bytes.len() {
            for end in start + 1..=bytes.len() {
                let expected = if ranges.contains(&(start..end)) {
                    Ok(ChunkOutcome::Duplicate)
                } else {
                    Err(StorageError::Conflict)
                };
                assert_eq!(upload.push(start as u64, &bytes[start..end]), expected);
            }
        }
        assert!(upload.is_complete());
        assert_eq!(upload.received_bytes(), 32);
        drop(upload);
        zero(&budget);
    }
}

#[test]
fn partial_overlap_conflicting_bytes_and_invalid_offsets_never_mutate_upload() {
    let (descriptor, bytes) = cloud();
    let budget = CpuBudget::new(1 << 20);
    let mut upload = PageUpload::new(&budget, descriptor).unwrap();
    upload.push(10, &bytes[10..20]).unwrap();
    let before = budget.usage();
    assert_eq!(upload.push(5, &bytes[5..15]), Err(StorageError::Conflict));
    assert_eq!(upload.push(15, &bytes[15..25]), Err(StorageError::Conflict));
    let mut conflict = bytes[10..20].to_vec();
    conflict[3] ^= 1;
    assert_eq!(upload.push(10, &conflict), Err(StorageError::Conflict));
    for (offset, chunk) in [
        (u64::MAX, &bytes[..1]),
        (bytes.len() as u64, &bytes[..1]),
        (0, &[][..]),
    ] {
        assert!(matches!(
            upload.push(offset, chunk),
            Err(StorageError::Invalid(_))
        ));
    }
    assert!(upload.push(0, &vec![0; DATA_CHUNK_BYTES + 1]).is_err());
    assert_eq!(upload.received_bytes(), 10);
    assert_eq!(budget.usage(), before);
    upload.push(20, &bytes[20..]).unwrap();
    upload.push(0, &bytes[..10]).unwrap();
    assert_eq!(
        upload.finish_cloud(&schema()).unwrap().page().bytes(),
        bytes
    );
    zero(&budget);
}

#[test]
fn one_byte_fragmentation_has_constant_index_memory_and_exact_staged_retries() {
    let (descriptor, bytes) = cloud();
    let budget = CpuBudget::new(1 << 20);
    let mut upload = PageUpload::new(&budget, descriptor).unwrap();
    let baseline = budget.usage();
    for i in (0..bytes.len()).rev() {
        upload.push(i as u64, &bytes[i..i + 1]).unwrap();
        assert_eq!(budget.usage(), baseline);
    }
    let staged = upload.finish_cloud(&schema()).unwrap();
    assert_eq!(budget.usage().reserved, 0);
    assert_eq!(budget.usage().scratch_bytes, 0);
    assert_eq!(
        budget.usage().upload_index_bytes,
        bytes.len().div_ceil(8) as u64
    );
    for i in 0..bytes.len() {
        assert_eq!(
            staged.retry(i as u64, &bytes[i..i + 1]).unwrap(),
            ChunkOutcome::Duplicate
        );
    }
    assert_eq!(staged.retry(0, &bytes[..2]), Err(StorageError::Conflict));
    let pointer = staged.page().bytes().as_ptr();
    let page = staged.into_page();
    assert_eq!(page.bytes().as_ptr(), pointer);
    assert_eq!(
        page.source_ids().unwrap().collect::<Vec<_>>(),
        [0, u64::MAX]
    );
    assert_eq!(page.column(0).unwrap().validity, [1]);
    assert!(page.column(2).is_none());
    assert!(page.voxel_keys().is_none());
    assert_eq!(budget.usage().upload_index_bytes, 0);
    drop(page);
    zero(&budget);
}

#[test]
fn staged_retries_reject_subranges_concatenations_and_conflicting_data() {
    let (descriptor, bytes) = cloud();
    let budget = CpuBudget::new(1 << 20);
    let mut upload = PageUpload::new(&budget, descriptor).unwrap();
    let ranges = [0..7, 7..39, 39..bytes.len()];
    for range in &ranges {
        upload
            .push(range.start as u64, &bytes[range.clone()])
            .unwrap();
    }
    let staged = upload.finish_cloud(&schema()).unwrap();
    for start in 0..bytes.len() {
        for end in start + 1..=bytes.len() {
            assert_eq!(
                staged.retry(start as u64, &bytes[start..end]),
                if ranges.contains(&(start..end)) {
                    Ok(ChunkOutcome::Duplicate)
                } else {
                    Err(StorageError::Conflict)
                }
            );
        }
    }
    let mut changed = bytes[..7].to_vec();
    changed[0] ^= 1;
    assert_eq!(staged.retry(0, &changed), Err(StorageError::Conflict));
    drop(staged);
    zero(&budget);
}

#[test]
fn incomplete_crc_kind_and_semantic_failures_release_every_reservation() {
    let budget = CpuBudget::new(1 << 20);
    let (descriptor, bytes) = cloud();
    let pending = PageUpload::new(&budget, descriptor.clone()).unwrap();
    assert!(matches!(
        pending.finish_cloud(&schema()),
        Err(StorageError::Incomplete)
    ));
    zero(&budget);
    let wrong = PageDescriptor {
        crc32: descriptor.crc32 ^ 1,
        ..descriptor.clone()
    };
    assert!(
        upload(&budget, wrong, &bytes)
            .finish_cloud(&schema())
            .is_err()
    );
    zero(&budget);
    assert!(
        upload(&budget, descriptor.clone(), &bytes)
            .finish_occupancy(&config())
            .is_err()
    );
    zero(&budget);
    let mut duplicate_ids = bytes.clone();
    duplicate_ids[bytes.len() - 8..].copy_from_slice(&0u64.to_le_bytes());
    let bad = PageDescriptor {
        crc32: page_crc32(&duplicate_ids),
        ..descriptor
    };
    assert!(
        upload(&budget, bad, &duplicate_ids)
            .finish_cloud(&schema())
            .is_err()
    );
    zero(&budget);
}

#[test]
fn immutable_page_pins_keep_one_charge_until_the_last_owner_drops() {
    let budget = CpuBudget::new(1 << 20);
    let (descriptor, bytes) = cloud();
    let page = upload(&budget, descriptor, &bytes)
        .finish_cloud(&schema())
        .unwrap()
        .into_page();
    let baseline = budget.usage();
    let pins: Vec<_> = (0..100).map(|_| page.clone()).collect();
    drop(page);
    assert_eq!(budget.usage(), baseline);
    assert_eq!(pins[99].bytes(), bytes);
    drop(pins);
    zero(&budget);
}

#[test]
fn packed_occupancy_keeps_signed_tiles_and_depth_21_keys_without_expansion() {
    let leaves = [
        leaf([i64::MIN, 0, i64::MAX], 21, (1 << 63) - 1),
        leaf([-1, 0, 0], 0, 0),
        leaf([0; 3], 1, 7),
        leaf([0; 3], 21, 3),
    ];
    let bytes = encode_occupancy_page(&config(), &leaves).unwrap();
    let descriptor = descriptor(&bytes, leaves.len() as u32, PageEncoding::OccupancyLeavesV1);
    let budget = CpuBudget::new(1 << 20);
    let page = upload(&budget, descriptor, &bytes)
        .finish_occupancy(&config())
        .unwrap()
        .into_page();
    assert_eq!(page.bytes(), bytes);
    assert_eq!(
        page.voxel_keys().unwrap().collect::<Vec<_>>(),
        leaves.map(|leaf| leaf.key)
    );
    assert!(page.source_ids().is_none());
    assert!(page.column(0).is_none());
    drop(page);
    zero(&budget);
}

#[test]
fn borrowed_validators_match_owned_decoders_under_byte_corruption() {
    let (_, bytes) = cloud();
    for index in 0..bytes.len() {
        for mask in [1, 0x80, 0xff] {
            let mut corrupt = bytes.clone();
            corrupt[index] ^= mask;
            assert_eq!(
                CloudPage::decode(&schema(), &corrupt).is_ok(),
                CloudPageView::validate(&schema(), &corrupt, &mut [0; 2]).is_ok(),
                "cloud {index} {mask}"
            );
        }
    }
    let leaves = [leaf([0; 3], 1, 0), leaf([0; 3], 2, 8)];
    let bytes = encode_occupancy_page(&config(), &leaves).unwrap();
    for index in 0..bytes.len() {
        for mask in [1, 0x80, 0xff] {
            let mut corrupt = bytes.clone();
            corrupt[index] ^= mask;
            assert_eq!(
                decode_occupancy_page(&config(), &corrupt).is_ok(),
                OccupancyPageView::validate(&config(), &corrupt, &mut [leaves[0].key; 2]).is_ok(),
                "octree {index} {mask}"
            );
        }
    }
}

#[test]
fn borrowed_occupancy_rejects_duplicate_and_ancestor_keys_in_all_orders() {
    for pair in [
        [leaf([0; 3], 0, 0), leaf([0; 3], 21, (1 << 63) - 1)],
        [leaf([-1; 3], 1, 7), leaf([-1; 3], 1, 7)],
    ] {
        for leaves in [pair, [pair[1], pair[0]]] {
            let mut bytes = encode_occupancy_page(&config(), &leaves[..1]).unwrap();
            bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
            bytes.extend_from_slice(&encode_occupancy_page(&config(), &leaves[1..]).unwrap()[16..]);
            assert!(OccupancyPageView::validate(&config(), &bytes, &mut [pair[0].key; 2]).is_err());
        }
    }
}

#[test]
fn real_page_publication_keeps_old_complete_bytes_on_failure_and_stale_commit() {
    let budget = CpuBudget::new(1 << 20);
    let (descriptor, bytes) = cloud();
    let key = DataKey::new("stream", "cloud").unwrap();
    let draw_key = (key.namespace.clone(), key.id.clone());
    let mut store = DrawingStore::<StoredPage>::new(8, 8).unwrap();
    let initial = store.begin_dataset(&key, DatasetKind::Cloud, None).unwrap();
    let page = upload(&budget, descriptor.clone(), &bytes)
        .finish_cloud(&schema())
        .unwrap()
        .into_page();
    let original = store.commit_dataset(initial, page.clone()).unwrap();
    let pinned_usage = budget.usage();
    let failed = upload(
        &budget,
        PageDescriptor {
            crc32: descriptor.crc32 ^ 1,
            ..descriptor.clone()
        },
        &bytes,
    );
    assert!(failed.finish_cloud(&schema()).is_err());
    assert!(Arc::ptr_eq(&store.get(&draw_key).unwrap().payload, &page));
    assert_eq!(budget.usage().allocated, pinned_usage.allocated);
    let first = store
        .begin_dataset(&key, DatasetKind::Cloud, Some(original))
        .unwrap();
    let stale = store
        .begin_dataset(&key, DatasetKind::Cloud, Some(original))
        .unwrap();
    let replacement = upload(&budget, descriptor, &bytes)
        .finish_cloud(&schema())
        .unwrap()
        .into_page();
    store.commit_dataset(first, replacement.clone()).unwrap();
    assert_eq!(
        store.commit_dataset(stale, page.clone()),
        Err(IdentityError::GenerationConflict)
    );
    assert!(Arc::ptr_eq(
        &store.get(&draw_key).unwrap().payload,
        &replacement
    ));
    drop(store);
    drop(replacement);
    assert_eq!(budget.usage().allocated, pinned_usage.allocated);
    assert_eq!(page.bytes(), bytes);
    drop(page);
    zero(&budget);
}

#[test]
fn maximum_point_page_uses_bounded_scratch_and_preserves_source_order() {
    let count = MAX_PAGE_POINTS as usize;
    let input = CloudPage {
        count: MAX_PAGE_POINTS,
        columns: vec![
            ColumnData {
                values: vec![0; count * 12],
                validity: vec![],
            },
            ColumnData {
                values: (0..count as u64).rev().flat_map(u64::to_le_bytes).collect(),
                validity: vec![],
            },
        ],
    };
    let bytes = input.encode(&schema()).unwrap();
    let descriptor = descriptor(&bytes, MAX_PAGE_POINTS, PageEncoding::CloudColumnsV1);
    let footprint = PageFootprint::for_descriptor(&descriptor).unwrap();
    let budget = CpuBudget::new(footprint.total());
    let mut pending = PageUpload::new(&budget, descriptor).unwrap();
    let chunks: Vec<_> = bytes.chunks(DATA_CHUNK_BYTES).enumerate().collect();
    for (index, chunk) in chunks.into_iter().rev() {
        pending
            .push((index * DATA_CHUNK_BYTES) as u64, chunk)
            .unwrap();
    }
    let page = pending.finish_cloud(&schema()).unwrap().into_page();
    assert_eq!(page.bytes(), bytes);
    assert_eq!(
        page.source_ids().unwrap().collect::<Vec<_>>(),
        (0..count as u64).rev().collect::<Vec<_>>()
    );
    assert_eq!(budget.usage().peak, footprint.total());
    assert_eq!(
        budget.usage().allocated,
        footprint.payload + footprint.metadata
    );
    drop(page);
    zero(&budget);
}

#[test]
fn empty_cloud_and_octree_pages_remain_valid_and_release_zero_length_scratch() {
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
    let page = upload(
        &budget,
        descriptor(&bytes, 0, PageEncoding::CloudColumnsV1),
        &bytes,
    )
    .finish_cloud(&schema())
    .unwrap()
    .into_page();
    assert_eq!(page.source_ids().unwrap().len(), 0);
    drop(page);
    zero(&budget);
    let bytes = encode_occupancy_page(&config(), &[]).unwrap();
    let page = upload(
        &budget,
        descriptor(&bytes, 0, PageEncoding::OccupancyLeavesV1),
        &bytes,
    )
    .finish_occupancy(&config())
    .unwrap()
    .into_page();
    assert_eq!(page.voxel_keys().unwrap().len(), 0);
    drop(page);
    zero(&budget);
}

#[test]
fn frozen_all_scalar_pages_pass_borrowed_validation_without_payload_copies() {
    let frozen: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/draw_data/wire-v1.json")).unwrap();
    for fixture in frozen["pages"].as_array().unwrap() {
        let bytes: Vec<_> = fixture["wire_hex"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|raw| u8::from_str_radix(std::str::from_utf8(raw).unwrap(), 16).unwrap())
            .collect();
        if fixture["schema"].is_object() {
            let schema = serde_json::from_value(fixture["schema"].clone()).unwrap();
            let view = CloudPageView::validate(&schema, &bytes, &mut [0; 2]).unwrap();
            let owned = CloudPage::decode(&schema, &bytes).unwrap();
            assert_eq!(view.count(), owned.count);
            let mut offset = 16 + 8 * view.columns().len();
            for (borrowed, owned) in view.columns().iter().zip(&owned.columns) {
                assert_eq!(borrowed.values, owned.values);
                assert_eq!(borrowed.validity, owned.validity);
                assert_eq!(borrowed.values.as_ptr(), bytes[offset..].as_ptr());
                offset += borrowed.values.len() + borrowed.validity.len();
            }
        } else {
            let config = serde_json::from_value(fixture["config"].clone()).unwrap();
            let expected = decode_occupancy_page(&config, &bytes).unwrap();
            let mut scratch = vec![
                VoxelKey {
                    tile: [0; 3],
                    depth: 0,
                    morton: 0
                };
                expected.len()
            ];
            let view = OccupancyPageView::validate(&config, &bytes, &mut scratch).unwrap();
            assert_eq!(view.leaves().collect::<Vec<_>>(), expected);
        }
    }
}

#[test]
fn mismatched_cloud_item_declaration_and_short_scratch_fail_without_leaks() {
    let (mut descriptor, bytes) = cloud();
    descriptor.items = 1;
    let budget = CpuBudget::new(1 << 20);
    assert!(
        upload(&budget, descriptor, &bytes)
            .finish_cloud(&schema())
            .is_err()
    );
    zero(&budget);
    assert!(CloudPageView::validate(&schema(), &bytes, &mut [0; 1]).is_err());
    let bytes = encode_occupancy_page(&config(), &[leaf([0; 3], 0, 0)]).unwrap();
    assert!(OccupancyPageView::validate(&config(), &bytes, &mut []).is_err());
}
