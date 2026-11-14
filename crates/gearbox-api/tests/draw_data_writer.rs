use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gearbox_api::wire::draw_data::*;
use gearbox_api::{
    Client, DataAdmission, DataPipelineConfig, DataPipelineError, DataUpload, DataUploadCleanup,
    DataUploadPage, DrawDataPipeline, IdentitySource,
};

#[allow(dead_code)]
#[path = "support/draw_data_upload.rs"]
mod fixture;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

fn setup() -> (gearbox_api::fake::FakeHost, DrawDataPipeline) {
    let host = gearbox_api::fake::FakeHost::start("writer-test", &[]).unwrap();
    let client =
        Client::connect_id(host.endpoint_id, IdentitySource::Random, "writer-client").unwrap();
    let pipeline = client
        .draw_data_pipeline(
            DataPipelineConfig {
                max_operations: 8,
                ..Default::default()
            },
            deadline(),
        )
        .unwrap();
    (host, pipeline)
}

fn pages(fixture: &fixture::UploadFixture) -> Vec<DataUploadPage<'_>> {
    fixture
        .pages
        .iter()
        .map(|(descriptor, bytes)| DataUploadPage { descriptor, bytes })
        .collect()
}

fn revision(reply: &DataReply) -> DatasetRevision {
    assert_eq!(reply.stage, data_stage::RETAINED);
    assert_eq!(reply.visible_generation, 0);
    DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    }
}

fn call(pipeline: &DrawDataPipeline, request: DataRequest) -> DataReply {
    let reply = pipeline
        .submit(&request, deadline(), DataAdmission::Wait)
        .unwrap()
        .wait(deadline())
        .unwrap();
    assert_eq!(
        reply.code,
        data_code::OK,
        "{}",
        String::from_utf8_lossy(&reply.message)
    );
    reply
}

#[test]
fn borrowed_pages_replace_and_delta_commit_with_one_writer() {
    let (_host, pipeline) = setup();
    let baseline = pipeline.stats().charged_bytes;
    let data = fixture::UploadFixture::cloud("writer", "cloud", 100_000).unwrap();
    let pages = pages(&data);
    let scans = [data.scan.clone()];
    let mut upload = DataUpload::replace(&data.descriptor, &scans, &pages);
    let mut writer = pipeline.writer(8, deadline()).unwrap();
    assert!(pipeline.stats().charged_bytes >= baseline + (2 << 20));
    let first = revision(&writer.upload(&upload, deadline(), None).unwrap());
    upload.base = Some(first);
    let second = revision(&writer.upload(&upload, deadline(), None).unwrap());
    assert_eq!(second.generation, first.generation + 1);
    upload.mode = CommitMode::Delta;
    upload.base = Some(second);
    upload.scans = &[];
    upload.pages = &pages[..1];
    let third = revision(&writer.upload(&upload, deadline(), None).unwrap());
    assert_eq!(third.generation, second.generation + 1);
    let mut connection = fixture::Connection::Pipeline {
        client: &pipeline,
        window: 8,
    };
    connection.verify(&data, third).unwrap();
    let mut moved = data.pages[0].0.clone();
    moved.address.page_id = 2;
    let replacement = [DataUploadPage {
        descriptor: &moved,
        bytes: &data.pages[0].1,
    }];
    let removed = [data.pages[0].0.address];
    upload.base = Some(third);
    upload.pages = &replacement;
    upload.removed_pages = &removed;
    let fourth = revision(&writer.upload(&upload, deadline(), None).unwrap());
    connection.verify(&data, fourth).unwrap();
    upload.base = Some(fourth);
    upload.pages = &[];
    upload.removed_pages = &[];
    upload.removed_scans = &[0];
    let empty = revision(&writer.upload(&upload, deadline(), None).unwrap());
    let listing = call(
        &pipeline,
        DataRequest {
            host_epoch: pipeline.host_epoch(),
            ..DataRequest::new(data_op::LIST)
        }
        .with_metadata(&DataListQuery {
            namespace: "writer".into(),
            ..Default::default()
        })
        .unwrap(),
    );
    let listing = DrawSummaryPage::from_json(&listing.metadata).unwrap();
    assert!(
        matches!(listing.items[0].object, SummaryOwner::Dataset { revision, scans: 0, pages: 0, source_samples: 0, payload_bytes: 0, .. } if revision == empty)
    );
    connection.clear("writer", writer.lease()).unwrap();
    let config = OctreeConfig {
        leaf_resolution_m: 0.05,
        max_depth: 12,
        free_max: 0.3,
        occupied_min: 0.7,
    };
    let bytes = encode_occupancy_page(
        &config,
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
    )
    .unwrap();
    let octree = DataDescriptor {
        key: DataKey::new("writer", "voxels").unwrap(),
        shape: DataShape::Octree {
            config,
            frame: DataFrame::default(),
        },
        presentation: Presentation::for_octree(),
        ..data.descriptor.clone()
    };
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
    let octree_pages = [DataUploadPage {
        descriptor: &page,
        bytes: &bytes,
    }];
    let retained = revision(
        &writer
            .upload(
                &DataUpload::replace(&octree, &[], &octree_pages),
                deadline(),
                None,
            )
            .unwrap(),
    );
    let listing = call(
        &pipeline,
        DataRequest {
            host_epoch: pipeline.host_epoch(),
            ..DataRequest::new(data_op::LIST)
        }
        .with_metadata(&DataListQuery {
            namespace: "writer".into(),
            ..Default::default()
        })
        .unwrap(),
    );
    let listing = DrawSummaryPage::from_json(&listing.metadata).unwrap();
    assert!(
        matches!(listing.items[0].object, SummaryOwner::Dataset { revision, scans: 0, pages: 1, payload_bytes, .. } if revision == retained && payload_bytes == bytes.len() as u64)
    );
    connection.clear("writer", writer.lease()).unwrap();
    writer.close(deadline()).unwrap();
    drop(writer);
    pipeline.close();
    assert_eq!(pipeline.stats().charged_bytes, baseline);
    assert_eq!(pipeline.stats().outstanding, 0);
}

#[test]
fn invalid_and_corrupt_uploads_preserve_previous_generation() {
    let (_host, pipeline) = setup();
    let data = fixture::UploadFixture::cloud("writer-errors", "cloud", 100).unwrap();
    let scans = [data.scan.clone()];
    let good = pages(&data);
    let mut writer = pipeline.writer(4, deadline()).unwrap();
    let retained = revision(
        &writer
            .upload(
                &DataUpload::replace(&data.descriptor, &scans, &good),
                deadline(),
                None,
            )
            .unwrap(),
    );
    let invalid = [DataUploadPage {
        descriptor: good[0].descriptor,
        bytes: &good[0].bytes[..8],
    }];
    let error = writer
        .upload(
            &DataUpload::replace(&data.descriptor, &scans, &invalid),
            deadline(),
            None,
        )
        .unwrap_err();
    assert!(matches!(error.cause, DataPipelineError::Invalid(_)));
    assert_eq!(error.cleanup, DataUploadCleanup::NotStarted);
    let mut bytes = good[0].bytes.to_vec();
    *bytes.last_mut().unwrap() ^= 1;
    let corrupt = [DataUploadPage {
        descriptor: good[0].descriptor,
        bytes: &bytes,
    }];
    let mut upload = DataUpload::replace(&data.descriptor, &scans, &corrupt);
    upload.base = Some(retained);
    let error = writer.upload(&upload, deadline(), None).unwrap_err();
    assert!(matches!(error.cause, DataPipelineError::Rejected { .. }));
    let mut connection = fixture::Connection::Pipeline {
        client: &pipeline,
        window: 4,
    };
    connection.verify(&data, retained).unwrap();
    let mut retry = DataUpload::replace(&data.descriptor, &scans, &good);
    retry.base = Some(retained);
    let next = revision(&writer.upload(&retry, deadline(), None).unwrap());
    assert_eq!(next.generation, retained.generation + 1);
    assert_eq!(pipeline.stats().outstanding, 0);
    connection.clear("writer-errors", writer.lease()).unwrap();
}

#[test]
fn cooperative_cancellation_stops_upload_without_retaining_partial_data() {
    let (host, pipeline) = setup();
    let data = fixture::UploadFixture::cloud("writer-cancel", "cloud", 100_000).unwrap();
    let pages = pages(&data);
    let scans = [data.scan.clone()];
    let upload = DataUpload::replace(&data.descriptor, &scans, &pages);
    let mut writer = pipeline.writer(8, deadline()).unwrap();
    let cancelled = AtomicBool::new(true);
    let error = writer
        .upload(&upload, deadline(), Some(&cancelled))
        .unwrap_err();
    assert_eq!(error.cleanup, DataUploadCleanup::NotStarted);
    cancelled.store(false, Ordering::Release);
    let baseline = pipeline.stats().finished;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let until = deadline();
            while pipeline.stats().finished < baseline + 4 {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            cancelled.store(true, Ordering::Release);
        });
        let error = writer
            .upload(&upload, deadline(), Some(&cancelled))
            .unwrap_err();
        assert_eq!(error.cause, DataPipelineError::Cancelled);
        assert_eq!(error.cleanup, DataUploadCleanup::Cancelled);
    });
    assert_eq!(
        host.scene
            .lock()
            .unwrap()
            .draws
            .registry()
            .lock()
            .unwrap()
            .dataset_len(),
        0
    );
    cancelled.store(false, Ordering::Release);
    revision(
        &writer
            .upload(&upload, deadline(), Some(&cancelled))
            .unwrap(),
    );
    assert_eq!(pipeline.stats().outstanding, 0);
}

#[test]
fn writer_workspace_is_charged_and_drop_releases_leases_and_queue_bytes() {
    let (_host, pipeline) = setup();
    let baseline = pipeline.stats().charged_bytes;
    assert!(matches!(
        pipeline.writer(0, deadline()),
        Err(DataPipelineError::Invalid(_))
    ));
    assert!(matches!(
        pipeline.writer(9, deadline()),
        Err(DataPipelineError::Invalid(_))
    ));
    for _ in 0..40 {
        let writer = pipeline.writer(1, deadline()).unwrap();
        assert!(pipeline.stats().charged_bytes >= baseline + (2 << 20));
        drop(writer);
    }
    let mut held = Vec::new();
    loop {
        match pipeline.writer(1, deadline()) {
            Ok(writer) => held.push(writer),
            Err(DataPipelineError::Full) => break,
            Err(error) => panic!("unexpected error: {error}"),
        }
    }
    assert!(!held.is_empty());
    assert!(pipeline.stats().charged_bytes <= pipeline.config().max_bytes);
    drop(held);
    pipeline.close();
    assert_eq!(pipeline.stats().charged_bytes, baseline);
    assert_eq!(pipeline.stats().outstanding, 0);
}

#[test]
fn closed_writer_and_expired_upload_do_not_publish() {
    let (_host, pipeline) = setup();
    let data = fixture::UploadFixture::cloud("writer-deadline", "cloud", 2).unwrap();
    let pages = pages(&data);
    let scans = [data.scan.clone()];
    let upload = DataUpload::replace(&data.descriptor, &scans, &pages);
    let mut writer = pipeline.writer(1, deadline()).unwrap();
    let submitted = pipeline.stats().submitted;
    let error = writer.upload(&upload, Instant::now(), None).unwrap_err();
    assert_eq!(
        error.cause,
        DataPipelineError::Deadline {
            may_have_been_sent: false
        }
    );
    assert_eq!(error.cleanup, DataUploadCleanup::NotStarted);
    assert_eq!(pipeline.stats().submitted, submitted);
    writer.close(deadline()).unwrap();
    let submitted = pipeline.stats().submitted;
    assert_eq!(
        writer.upload(&upload, deadline(), None).unwrap_err().cause,
        DataPipelineError::Closed
    );
    writer.close(deadline()).unwrap();
    assert_eq!(pipeline.stats().submitted, submitted);
}

#[test]
fn sender_admission_keeps_room_for_its_acquisition_result() {
    let (host, baseline) = setup();
    let budget = baseline.stats().charged_bytes + (2 << 20);
    drop(baseline);
    let client =
        Client::connect_id(host.endpoint_id, IdentitySource::Random, "tight-writer").unwrap();
    let pipeline = client
        .draw_data_pipeline(
            DataPipelineConfig {
                max_bytes: budget,
                max_operations: 8,
                ..Default::default()
            },
            deadline(),
        )
        .unwrap();
    let before = pipeline.stats();
    assert!(matches!(
        pipeline.writer(1, deadline()),
        Err(DataPipelineError::Full)
    ));
    assert_eq!(pipeline.stats(), before);
}

#[test]
fn cancel_reports_already_retained_when_commit_acknowledgement_is_invalid() {
    let host =
        gearbox_api::HostBus::new(gearbox_api::HostConfig::new("writer-ack", "test").ephemeral())
            .unwrap();
    let client =
        Client::connect_id(host.endpoint_id(), IdentitySource::Random, "writer-client").unwrap();
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let lane = host.draw_data_lane();
            let mut store =
                gearbox_api::draw_data::StreamingStore::new(lane.epoch(), DataLimits::reference())
                    .unwrap();
            while !stop.load(Ordering::Acquire) {
                lane.serve(&store.capabilities(), |request, _| {
                    let mut reply = store.handle(&request, 0);
                    if request.operation == data_op::COMMIT && reply.code == data_code::OK {
                        reply.stage = data_stage::RECEIVED;
                        reply.incarnation = 0;
                        reply.stored_generation = 0;
                    }
                    reply
                })
                .unwrap();
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let pipeline = client
                .draw_data_pipeline(DataPipelineConfig::default(), deadline())
                .unwrap();
            let data = fixture::UploadFixture::cloud("writer-ack", "cloud", 10).unwrap();
            let pages = pages(&data);
            let scans = [data.scan.clone()];
            let mut writer = pipeline.writer(4, deadline()).unwrap();
            let error = writer
                .upload(
                    &DataUpload::replace(&data.descriptor, &scans, &pages),
                    deadline(),
                    None,
                )
                .unwrap_err();
            assert!(matches!(error.cause, DataPipelineError::Protocol(_)));
            let DataUploadCleanup::Retained(retained) = error.cleanup else {
                panic!("commit was incorrectly reported rolled back");
            };
            fixture::Connection::Pipeline {
                client: &pipeline,
                window: 4,
            }
            .verify(&data, retained)
            .unwrap();
            call(
                &pipeline,
                DataRequest {
                    host_epoch: pipeline.host_epoch(),
                    lease: writer.lease(),
                    ..DataRequest::new(data_op::CLEAR)
                }
                .with_metadata(&NamespaceFilter {
                    namespace: "writer-ack".into(),
                })
                .unwrap(),
            );
        }));
        stop.store(true, Ordering::Release);
        if let Err(error) = run {
            std::panic::resume_unwind(error);
        }
    });
}

#[test]
fn owned_writer_keeps_pipeline_alive_until_lease_owner_drops() {
    let (_host, pipeline) = setup();
    let pipeline = std::sync::Arc::new(pipeline);
    let weak = std::sync::Arc::downgrade(&pipeline);
    let mut writer = pipeline.writer_owned(4, deadline()).unwrap();
    drop(pipeline);
    let data = fixture::UploadFixture::cloud("owned-writer", "cloud", 4).unwrap();
    let pages = pages(&data);
    let scans = [data.scan.clone()];
    revision(
        &writer
            .upload(
                &DataUpload::replace(&data.descriptor, &scans, &pages),
                deadline(),
                None,
            )
            .unwrap(),
    );
    let pipeline = weak.upgrade().unwrap();
    fixture::Connection::Pipeline {
        client: &pipeline,
        window: 4,
    }
    .clear("owned-writer", writer.lease())
    .unwrap();
    drop(pipeline);
    writer.close(deadline()).unwrap();
    drop(writer);
    assert!(weak.upgrade().is_none());
}
