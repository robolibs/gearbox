use gearbox_api::wire::{clear_scope, draw_data::*};
use gearbox_api::{Client, DrawDataClient, DrawEntity};
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "draw_data_contracts.rs"]
mod contracts;

#[path = "draw_data_compatibility.rs"]
mod compatibility;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn checked(data: &mut DrawDataClient, request: &DataRequest) -> Result<DataReply> {
    let reply = data.request(request)?;
    if reply.code != data_code::OK {
        return Err(format!(
            "operation {} failed {}: {}",
            request.operation,
            reply.code,
            String::from_utf8_lossy(&reply.message)
        )
        .into());
    }
    Ok(reply)
}

fn request(op: u32, epoch: u64, lease: u64) -> DataRequest {
    DataRequest {
        host_epoch: epoch,
        lease,
        ..DataRequest::new(op)
    }
}

fn begin(
    epoch: u64,
    lease: u64,
    id: u64,
    descriptor: &DataDescriptor,
    base: Option<DatasetRevision>,
    mode: CommitMode,
    scans: u32,
    pages: u32,
    bytes: u64,
) -> DataRequest {
    DataRequest {
        transaction: id,
        incarnation: base.map_or(0, |r| r.incarnation),
        base_generation: base.map_or(0, |r| r.generation),
        total_bytes: bytes,
        ..request(data_op::BEGIN, epoch, lease)
    }
    .with_key(&descriptor.key)
    .unwrap()
    .with_metadata(&BeginMetadata {
        descriptor: descriptor.clone(),
        mode,
        scans,
        pages,
        removed_pages: 0,
        removed_scans: 0,
    })
    .unwrap()
}

fn edit(begin: &DataRequest, op: u32) -> DataRequest {
    DataRequest {
        transaction: begin.transaction,
        key: begin.key.clone(),
        incarnation: begin.incarnation,
        base_generation: begin.base_generation,
        ..request(op, begin.host_epoch, begin.lease)
    }
}

fn revision(reply: &DataReply) -> DatasetRevision {
    DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    }
}

fn status(data: &mut DrawDataClient, epoch: u64, key: &DataKey) -> Result<DatasetStatus> {
    let rendering = !data.capabilities().rendered.is_empty();
    let request = request(data_op::STATUS, epoch, 0)
        .with_key(key)?
        .with_metadata(&DataStatusQuery::default())?;
    let reply = checked(data, &request)?;
    ensure(
        rendering || (reply.stage != data_stage::VISIBLE && reply.visible_generation == 0),
        "backend falsely acknowledged rendering",
    )?;
    Ok(DatasetStatus::from_json(&reply.metadata)?)
}

fn upload(
    data: &mut DrawDataClient,
    begin: &DataRequest,
    page: &PageDescriptor,
    bytes: &[u8],
) -> Result<()> {
    let declare = DataRequest {
        scan_key: page.address.scan_key,
        page_id: page.address.page_id,
        ..edit(begin, data_op::DECLARE_PAGE)
    }
    .with_metadata(page)?;
    checked(data, &declare)?;
    let middle = bytes.len() / 2;
    for (offset, part) in [
        (middle, &bytes[middle..]),
        (0, &bytes[..middle]),
        (middle, &bytes[middle..]),
    ] {
        let request = DataRequest {
            scan_key: page.address.scan_key,
            page_id: page.address.page_id,
            offset: offset as u64,
            payload: part.to_vec(),
            ..edit(begin, data_op::CHUNK)
        };
        ensure(
            checked(data, &request)?.stage == data_stage::RECEIVED,
            "chunk was not acknowledged as received",
        )?;
    }
    Ok(())
}

fn unchecked(client: &Client, topic: &str, request: &DataRequest) -> Result<DataReply> {
    use gearbox_api::wire::{Env, pack};
    let mut transport = client
        .agent
        .by_id(client.host.ok_or("host not selected")?)?
        .req_client::<Env, Env>(topic)?;
    let reply = transport.call(&pack(request))?;
    Ok(DataReply::unpack_checked(
        reply.header().type_hash,
        reply.payload(),
    )?)
}

fn terminal_failures(
    client: &Client,
    data: &mut DrawDataClient,
    descriptor: &DataDescriptor,
    retained: DatasetRevision,
    page: &PageDescriptor,
    bytes: &[u8],
) -> Result<()> {
    use gearbox_api::topics;
    let epoch = data.capabilities().host_epoch;
    let lease = checked(data, &request(data_op::OPEN_WRITER, epoch, 0))?.lease;
    let baseline = status(data, epoch, &descriptor.key)?
        .resources
        .resident_page_bytes;
    for transaction in 1..=3 {
        let start = begin(
            epoch,
            lease,
            transaction,
            descriptor,
            Some(retained),
            CommitMode::Replace,
            0,
            1,
            page.byte_len,
        );
        checked(data, &start)?;
        checked(
            data,
            &DataRequest {
                scan_key: page.address.scan_key,
                page_id: page.address.page_id,
                ..edit(&start, data_op::DECLARE_PAGE)
            }
            .with_metadata(page)?,
        )?;
        let middle = bytes.len() / 2;
        let chunk = DataRequest {
            scan_key: page.address.scan_key,
            page_id: page.address.page_id,
            payload: bytes[..middle].to_vec(),
            ..edit(&start, data_op::CHUNK)
        };
        ensure(
            unchecked(client, topics::DRAW_DATA_STATUS, &chunk)?.code == data_code::INVALID,
            "wrong route accepted a mutation",
        )?;
        let stale = DataRequest {
            host_epoch: if epoch == 1 { 2 } else { 1 },
            ..chunk.clone()
        };
        ensure(
            unchecked(client, topics::DRAW_DATA, &stale)?.code == data_code::STALE_EPOCH,
            "stale host accepted a chunk",
        )?;
        checked(data, &chunk)?;
        let (bad, code, phase) = match transaction {
            1 => (
                DataRequest {
                    flags: 1,
                    ..chunk.clone()
                },
                data_code::INVALID,
                DataPhase::Receiving,
            ),
            2 => {
                let mut payload = bytes[middle..].to_vec();
                *payload.last_mut().unwrap() ^= 1;
                (
                    DataRequest {
                        offset: middle as u64,
                        payload,
                        ..chunk.clone()
                    },
                    data_code::INVALID,
                    DataPhase::Validating,
                )
            }
            _ => (
                DataRequest {
                    offset: 1,
                    payload: bytes[1..middle].to_vec(),
                    ..chunk.clone()
                },
                data_code::CONFLICT,
                DataPhase::Receiving,
            ),
        };
        ensure(
            unchecked(client, topics::DRAW_DATA, &bad)?.code == code,
            "invalid chunk did not fail explicitly",
        )?;
        ensure(
            data.request(&edit(&start, data_op::COMMIT))?.code == code,
            "failed upload was not terminal",
        )?;
        let current = status(data, epoch, &descriptor.key)?;
        ensure(
            current.retained == Some(retained) && current.transfer.is_none(),
            "failed upload changed the retained generation",
        )?;
        ensure(
            current
                .failure
                .as_ref()
                .is_some_and(|failure| failure.code == code && failure.phase == phase),
            "failure phase was lost",
        )?;
        ensure(
            current.resources.pending_transactions == 0
                && current.resources.incomplete_bytes == 0
                && current.resources.staged_bytes == 0
                && current.resources.resident_page_bytes == baseline,
            "failed upload leaked payload allocation",
        )?;
    }
    checked(data, &request(data_op::CLOSE_WRITER, epoch, lease))?;
    Ok(())
}

fn accumulation(data: &mut DrawDataClient, template: &DataDescriptor) -> Result<()> {
    let epoch = data.capabilities().host_epoch;
    ensure(
        data.capabilities().features.accumulation,
        "accumulation capability missing",
    )?;
    let before = status(data, epoch, &template.key)?.counters;
    let lease = checked(data, &request(data_op::OPEN_WRITER, epoch, 0))?.lease;
    let mut descriptor = template.clone();
    descriptor.key.id = "accumulated".into();
    descriptor.stream = StreamMode::Accumulate {
        max_scans: 2,
        max_age_s: 2.0,
        max_bytes: 4096,
        age_clock: AgeClock::Source,
    };
    let DataShape::Cloud { schema } = &descriptor.shape else {
        unreachable!()
    };
    let mut base = None;
    for (id, seconds, count, expected) in [(1, 1, 1, 1), (2, 2, 2, 3), (3, 3, 4, 6), (4, 1, 8, 6)] {
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
        .encode(schema)?;
        let page = PageDescriptor {
            address: PageAddress {
                scan_key: id,
                page_id: 1,
            },
            encoding: PageEncoding::CloudColumnsV1,
            items: count,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [0.0; 3],
        };
        let start = begin(
            epoch,
            lease,
            id,
            &descriptor,
            base,
            if base.is_some() {
                CommitMode::Delta
            } else {
                CommitMode::Replace
            },
            1,
            1,
            page.byte_len,
        );
        checked(data, &start)?;
        let mut scan = contracts::scan(id);
        scan.source_count = u64::from(count);
        scan.organization = None;
        scan.frame.capture.as_mut().unwrap().source_time_ns = Some(seconds * 1_000_000_000);
        scan.frame.capture_transform.as_mut().unwrap().stamp = scan.frame.capture.clone().unwrap();
        checked(
            data,
            &DataRequest {
                scan_key: id,
                ..edit(&start, data_op::DECLARE_SCAN)
            }
            .with_metadata(&scan)?,
        )?;
        upload(data, &start, &page, &bytes)?;
        let retained = checked(data, &edit(&start, data_op::COMMIT))?;
        base = Some(revision(&retained));
        let retry = checked(data, &edit(&start, data_op::COMMIT))?;
        ensure(
            revision(&retry) == base.unwrap(),
            "accumulation retry republished",
        )?;
        let query = request(data_op::LIST, epoch, 0).with_metadata(&DataListQuery {
            namespace: descriptor.key.namespace.clone(),
            limit: 128,
            cursor: None,
        })?;
        let listing = DrawSummaryPage::from_json(&checked(data, &query)?.metadata)?;
        ensure(listing.items.iter().any(|item| item.id == descriptor.key.id && matches!(item.object, SummaryOwner::Dataset { source_samples, scans, .. } if source_samples == expected && scans == if id == 1 { 1 } else { 2 })), "whole-scan accumulation retained incorrect samples")?;
    }
    let after = status(data, epoch, &descriptor.key)?;
    ensure(
        after.counters.evicted_scans == before.evicted_scans + 1
            && after.counters.dropped_scans == before.dropped_scans + 1,
        "eviction/drop counters lost or repeated",
    )?;
    checked(
        data,
        &request(data_op::DELETE, epoch, lease).with_key(&descriptor.key)?,
    )?;
    checked(data, &request(data_op::CLOSE_WRITER, epoch, lease))?;
    Ok(())
}

pub fn verify(client: &Client, reset_owned_scene: bool) -> Result<serde_json::Value> {
    let namespace = format!("draw-data-probe/{}", std::process::id());
    let mut data = client.draw_data()?;
    let caps = data.capabilities().clone();
    let rendering = !caps.rendered.is_empty();
    ensure(
        caps.retained.starts_with(&[DatasetKind::Cloud, DatasetKind::Octree])
            && (caps.rendered.is_empty() || caps.rendered == caps.retained),
        "incorrect data retention/rendering capabilities",
    )?;
    ensure(
        caps.features.atomic_generations
            && caps.features.delta_updates
            && caps.features.presentation_updates
            && caps.features.unified_listing,
        "missing integrated storage features",
    )?;
    let epoch = caps.host_epoch;
    let lease = checked(&mut data, &request(data_op::OPEN_WRITER, epoch, 0))?.lease;
    ensure(lease != 0, "writer lease missing")?;
    ensure(
        client.draw_clear(&namespace)?.is_ok(),
        "initial namespace clear failed",
    )?;
    ensure(
        client
            .draw(&DrawEntity::points(&namespace, "legacy", &[[1.0; 3]]))?
            .is_ok(),
        "legacy drawing failed",
    )?;
    let mut schema = contracts::schema();
    schema.attributes.truncate(2);
    let descriptor = DataDescriptor {
        key: DataKey::new(&namespace, "typed")?,
        shape: DataShape::Cloud {
            schema: schema.clone(),
        },
        stream: StreamMode::RetainedMap,
        ..contracts::cloud()
    };
    let bytes = CloudPage {
        count: 2,
        columns: vec![
            ColumnData {
                values: [1.0f64, 2.0, 3.0, 4.0, 5.0, 6.0]
                    .into_iter()
                    .flat_map(f64::to_le_bytes)
                    .collect(),
                validity: vec![],
            },
            ColumnData {
                values: [0u64, u64::MAX]
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect(),
                validity: vec![],
            },
        ],
    }
    .encode(&schema)?;
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 1,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: 2,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let first = begin(
        epoch,
        lease,
        1,
        &descriptor,
        None,
        CommitMode::Replace,
        1,
        1,
        page.byte_len,
    );
    checked(&mut data, &first)?;
    checked(&mut data, &first)?;
    let scan = ScanDescriptor {
        source_count: 2,
        organization: None,
        ..contracts::scan(0)
    };
    checked(
        &mut data,
        &edit(&first, data_op::DECLARE_SCAN).with_metadata(&scan)?,
    )?;
    upload(&mut data, &first, &page, &bytes)?;
    let pending = status(&mut data, epoch, &descriptor.key)?;
    ensure(
        pending.retained.is_none()
            && pending
                .transfer
                .as_ref()
                .is_some_and(|progress| progress.staged_bytes == page.byte_len),
        "pending upload became retained early or lost progress",
    )?;
    let retained = checked(&mut data, &edit(&first, data_op::COMMIT))?;
    ensure(
        retained.stage == data_stage::RETAINED,
        "commit did not retain",
    )?;
    let duplicate = checked(&mut data, &edit(&first, data_op::COMMIT))?;
    ensure(
        retained.pack_checked()?.wire() == duplicate.pack_checked()?.wire(),
        "commit retry changed its outcome",
    )?;
    ensure(
        !client
            .draw(&DrawEntity::points(&namespace, "typed", &[[0.0; 3]]))?
            .is_ok(),
        "legacy mutation replaced a dataset",
    )?;
    compatibility::verify(client, &mut data, &descriptor, revision(&retained))?;
    let list = request(data_op::LIST, epoch, 0).with_metadata(&DataListQuery {
        namespace: namespace.clone(),
        limit: 1,
        cursor: None,
    })?;
    let listing = DrawSummaryPage::from_json(&checked(&mut data, &list)?.metadata)?;
    ensure(
        listing.items.len() == 1 && matches!(listing.items[0].object, SummaryOwner::Legacy { .. }),
        "mixed list omitted legacy ownership",
    )?;
    let next = request(data_op::LIST, epoch, 0).with_metadata(&DataListQuery {
        namespace: namespace.clone(),
        limit: 1,
        cursor: listing.next_cursor.clone(),
    })?;
    let second = DrawSummaryPage::from_json(&checked(&mut data, &next)?.metadata)?;
    ensure(
        second.items.len() == 1
            && matches!(
                second.items[0].object,
                SummaryOwner::Dataset {
                    source_samples: 2,
                    valid_samples: 2,
                    ..
                }
            ),
        "mixed list omitted retained data or counts",
    )?;
    ensure(
        second.to_json()?
            == DrawSummaryPage::from_json(&checked(&mut data, &next)?.metadata)?.to_json()?,
        "cursor retry changed its snapshot",
    )?;
    let before = status(&mut data, epoch, &descriptor.key)?;
    ensure(
        rendering != matches!(before.visibility, GenerationVisibility::Unavailable { .. }),
        "visibility disagrees with the negotiated renderer",
    )?;
    let mut presentation = descriptor.presentation.clone();
    presentation.blur = 0.8;
    presentation.color = ColorMode::Constant {
        rgba: [240, 50, 140, 210],
    };
    let style = DataRequest {
        incarnation: retained.incarnation,
        base_generation: retained.stored_generation,
        ..request(data_op::PATCH_PRESENTATION, epoch, lease)
    }
    .with_key(&descriptor.key)?
    .with_metadata(&presentation)?;
    let styled = checked(&mut data, &style)?;
    ensure(
        styled.stored_generation == retained.stored_generation + 1,
        "presentation did not publish a new generation",
    )?;
    ensure(
        status(&mut data, epoch, &descriptor.key)?
            .resources
            .resident_page_bytes
            == before.resources.resident_page_bytes,
        "presentation duplicated page payloads",
    )?;
    ensure(
        data.request(&next)?.code == data_code::STALE_CURSOR,
        "mutation did not invalidate listing cursor",
    )?;
    let stale = begin(
        epoch,
        lease,
        2,
        &descriptor,
        Some(revision(&retained)),
        CommitMode::Delta,
        0,
        0,
        0,
    );
    ensure(
        data.request(&stale)?.code == data_code::CONFLICT,
        "stale delta was accepted",
    )?;
    let delta = begin(
        epoch,
        lease,
        3,
        &descriptor,
        Some(revision(&styled)),
        CommitMode::Delta,
        0,
        0,
        0,
    );
    checked(&mut data, &delta)?;
    let current = checked(&mut data, &edit(&delta, data_op::COMMIT))?;
    let delayed = begin(
        epoch,
        lease,
        4,
        &descriptor,
        Some(revision(&current)),
        CommitMode::Delta,
        0,
        0,
        0,
    );
    checked(&mut data, &delayed)?;
    ensure(
        client.draw_delete(&namespace, "typed")?.is_ok(),
        "legacy delete failed",
    )?;
    ensure(
        data.request(&edit(&delayed, data_op::COMMIT))?.code == data_code::HISTORY_EXPIRED,
        "late commit resurrected deleted data",
    )?;
    let recreate = begin(
        epoch,
        lease,
        5,
        &descriptor,
        None,
        CommitMode::Replace,
        0,
        0,
        0,
    );
    checked(&mut data, &recreate)?;
    let fresh = checked(&mut data, &edit(&recreate, data_op::COMMIT))?;
    ensure(
        fresh.incarnation != retained.incarnation && fresh.stored_generation == 1,
        "delete/recreate reused the old incarnation",
    )?;
    let mut octree = contracts::octree();
    octree.key = DataKey::new(&namespace, "voxels")?;
    let DataShape::Octree { config, .. } = &octree.shape else {
        unreachable!()
    };
    let bytes = encode_occupancy_page(
        config,
        &[OccupancyLeaf {
            key: VoxelKey {
                tile: [-2, 0, 1],
                depth: 1,
                morton: 7,
            },
            value: OccupancyValue {
                probability: Some(0.9),
                confidence: Some(0.7),
                class_id: Some(4),
            },
        }],
    )?;
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding: PageEncoding::OccupancyLeavesV1,
        items: 1,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let voxels = begin(
        epoch,
        lease,
        6,
        &octree,
        None,
        CommitMode::Replace,
        0,
        1,
        page.byte_len,
    );
    checked(&mut data, &voxels)?;
    upload(&mut data, &voxels, &page, &bytes)?;
    let occupancy = checked(&mut data, &edit(&voxels, data_op::COMMIT))?;
    ensure(
        status(&mut data, epoch, &octree.key)?.kind == DatasetKind::Octree,
        "occupancy dataset did not retain",
    )?;
    terminal_failures(
        client,
        &mut data,
        &octree,
        revision(&occupancy),
        &page,
        &bytes,
    )?;
    let delete_legacy =
        request(data_op::DELETE, epoch, lease).with_key(&DataKey::new(&namespace, "legacy")?)?;
    checked(&mut data, &delete_legacy)?;
    ensure(
        client.draw_list(&namespace)?.is_empty(),
        "typed delete did not remove legacy ownership",
    )?;
    accumulation(&mut data, &descriptor)?;
    if reset_owned_scene {
        let pending = begin(
            epoch,
            lease,
            8,
            &descriptor,
            Some(revision(&fresh)),
            CommitMode::Delta,
            0,
            0,
            0,
        );
        checked(&mut data, &pending)?;
        ensure(
            client.clear(clear_scope::MARKERS)?.is_ok(),
            "scene marker reset failed",
        )?;
        let request = request(data_op::STATUS, epoch, 0)
            .with_key(&descriptor.key)?
            .with_metadata(&DataStatusQuery::default())?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if data.request(&request)?.code == data_code::NOT_FOUND {
                break;
            }
            ensure(
                Instant::now() < deadline,
                "scene reset did not remove retained data",
            )?;
            std::thread::sleep(Duration::from_millis(10));
        }
        ensure(
            data.request(&edit(&pending, data_op::COMMIT))?.code == data_code::HISTORY_EXPIRED,
            "scene reset did not revoke pending data",
        )?;
    }
    ensure(
        client.draw_clear(&namespace)?.is_ok(),
        "final namespace cleanup failed",
    )?;
    checked(&mut data, &request(data_op::CLOSE_WRITER, epoch, lease))?;
    Ok(
        serde_json::json!({"result":"native retained cloud and occupancy lifecycle passed", "namespace":namespace,
        "reset_verified":reset_owned_scene,"rendered":rendering,"capabilities":caps,
        "checks":["received versus retained", "reordered chunks and retries", "exact commit retry", "legacy ownership conflict",
            "bounded mixed listing", "cursor retry and invalidation", "renderer-consistent visibility", "shared presentation pages",
            "stale and shared delta", "legacy delete barrier", "new incarnation", "occupancy retention", "typed delete of legacy",
            "terminal invalid flags", "terminal checksum failure", "terminal overlap conflict", "wrong route and epoch preserve upload",
            "whole-scan accumulation", "source age and late capture", "eviction counters and retries", "namespace cleanup",
            "legacy kind transitions", "all legacy mutations reject retained and reserved data", "pending legacy ownership",
            "legacy list excludes datasets", "dataset kind replacement requires delete", "namespace clear and late chunks"]}),
    )
}
