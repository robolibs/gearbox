use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gearbox_api::wire::draw_data::*;
use gearbox_api::{
    Client, DataAdmission, DataCancelOutcome, DataPipelineConfig, DataPipelineError,
    DataRequestTicket, DrawDataPipeline, HostBus, HostConfig, IdentitySource,
};

#[allow(dead_code)]
#[path = "support/draw_data_contracts.rs"]
mod contracts;

#[allow(dead_code)]
#[path = "support/draw_data_upload.rs"]
mod upload;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn until(mut condition: impl FnMut() -> bool) {
    let end = deadline();
    while !condition() {
        assert!(Instant::now() < end, "condition did not become true");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[derive(Default)]
struct Gate {
    entered: AtomicBool,
    released: Mutex<bool>,
    ready: Condvar,
}

impl Gate {
    fn enter(&self) {
        self.entered.store(true, Ordering::Release);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.ready.wait(released).unwrap();
        }
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.ready.notify_all();
    }
}

struct Harness {
    client: Client,
    gate: Arc<Gate>,
    events: Arc<Mutex<Vec<(u32, u64)>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Harness {
    fn new(gated: Option<u32>) -> Self {
        let host = HostBus::new(HostConfig::new("pipeline-test", "test").ephemeral()).unwrap();
        let client = Client::connect_id(
            host.endpoint_id(),
            IdentitySource::Random,
            "pipeline-client",
        )
        .unwrap();
        let gate = Arc::new(Gate::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let gate = gate.clone();
            let events = events.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let lane = host.draw_data_lane();
                let caps = DataCapabilities::wire_only(lane.epoch());
                let mut held = false;
                while !stop.load(Ordering::Acquire) {
                    let _ = lane.serve(&caps, |request, _| {
                        events
                            .lock()
                            .unwrap()
                            .push((request.operation, request.offset));
                        if gated == Some(request.operation) && !held {
                            held = true;
                            gate.enter();
                        }
                        DataReply {
                            lease: if request.operation == data_op::OPEN_WRITER {
                                1
                            } else {
                                0
                            },
                            ..DataReply::new(caps.host_epoch)
                        }
                    });
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
        };
        Self {
            client,
            gate,
            events,
            stop,
            worker: Some(worker),
        }
    }

    fn pipeline(&self, workers: usize, operations: usize) -> DrawDataPipeline {
        self.client
            .draw_data_pipeline(
                DataPipelineConfig {
                    parallel_requests: workers,
                    max_operations: operations,
                    ..Default::default()
                },
                deadline(),
            )
            .unwrap()
    }

    fn entered(&self) {
        until(|| self.gate.entered.load(Ordering::Acquire));
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.gate.release();
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn open(pipeline: &DrawDataPipeline) -> DataRequest {
    DataRequest {
        host_epoch: pipeline.host_epoch(),
        ..DataRequest::new(data_op::OPEN_WRITER)
    }
}

fn chunk(pipeline: &DrawDataPipeline, offset: u64, bytes: usize) -> DataRequest {
    DataRequest {
        host_epoch: pipeline.host_epoch(),
        lease: 1,
        transaction: 1,
        page_id: 1,
        offset,
        payload: vec![7; bytes],
        ..DataRequest::new(data_op::CHUNK)
    }
    .with_key(&DataKey::new("pipeline", "cloud").unwrap())
    .unwrap()
}

fn submit(pipeline: &DrawDataPipeline, request: DataRequest) -> DataRequestTicket {
    pipeline
        .submit(&request, deadline(), DataAdmission::Reject)
        .unwrap()
}

fn ok(ticket: &mut DataRequestTicket) -> DataReply {
    let reply = ticket.wait(deadline()).unwrap();
    assert_eq!(
        reply.code,
        data_code::OK,
        "{}",
        String::from_utf8_lossy(&reply.message)
    );
    reply
}

#[test]
fn chunks_overlap_but_delete_and_clear_are_fifo_barriers() {
    let harness = Harness::new(Some(data_op::CHUNK));
    let pipeline = harness.pipeline(4, 16);
    let mut tickets = vec![submit(&pipeline, chunk(&pipeline, 0, 32))];
    harness.entered();
    for index in 1..4 {
        tickets.push(submit(&pipeline, chunk(&pipeline, index * 32, 32)));
    }
    until(|| pipeline.stats().running == 4);
    let delete = DataRequest {
        host_epoch: pipeline.host_epoch(),
        lease: 1,
        ..DataRequest::new(data_op::DELETE)
    }
    .with_key(&DataKey::new("pipeline", "cloud").unwrap())
    .unwrap();
    tickets.push(submit(&pipeline, delete));
    tickets.push(submit(&pipeline, chunk(&pipeline, 128, 32)));
    let clear = DataRequest {
        host_epoch: pipeline.host_epoch(),
        lease: 1,
        ..DataRequest::new(data_op::CLEAR)
    }
    .with_metadata(&NamespaceFilter {
        namespace: "pipeline".into(),
    })
    .unwrap();
    tickets.push(submit(&pipeline, clear));
    tickets.push(submit(&pipeline, chunk(&pipeline, 160, 32)));
    assert_eq!(pipeline.stats().queued, 4);
    harness.gate.release();
    for ticket in &mut tickets {
        ok(ticket);
    }
    pipeline.close();
    let operations: Vec<_> = harness
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|event| event.0)
        .collect();
    assert_eq!(
        operations,
        vec![
            data_op::CHUNK,
            data_op::CHUNK,
            data_op::CHUNK,
            data_op::CHUNK,
            data_op::DELETE,
            data_op::CHUNK,
            data_op::CLEAR,
            data_op::CHUNK
        ]
    );
    assert_eq!(pipeline.stats().outstanding, 0);
}

#[test]
fn completed_unconsumed_results_keep_operation_slots() {
    let harness = Harness::new(None);
    let pipeline = harness.pipeline(1, 2);
    let baseline = pipeline.stats().charged_bytes;
    let mut first = submit(&pipeline, open(&pipeline));
    let second = submit(&pipeline, open(&pipeline));
    until(|| pipeline.stats().finished == 2);
    assert!(matches!(
        pipeline.submit(&open(&pipeline), deadline(), DataAdmission::Reject),
        Err(DataPipelineError::Full)
    ));
    assert_eq!(pipeline.stats().outstanding, 2);
    assert!(pipeline.stats().charged_bytes > baseline);
    drop(second);
    until(|| pipeline.stats().outstanding == 1);
    let mut third = submit(&pipeline, open(&pipeline));
    ok(&mut first);
    ok(&mut third);
    assert_eq!(pipeline.stats().outstanding, 0);
    until(|| pipeline.stats().outstanding == 0);
    assert_eq!(pipeline.stats().charged_bytes, baseline);
    assert!(matches!(
        first.try_take(),
        Some(Err(DataPipelineError::Consumed))
    ));
}

#[test]
fn queued_cancel_frees_payload_and_never_publishes() {
    let harness = Harness::new(Some(data_op::OPEN_WRITER));
    let pipeline = harness.pipeline(1, 4);
    let mut first = submit(&pipeline, open(&pipeline));
    harness.entered();
    assert_eq!(first.cancel(), DataCancelOutcome::InFlight);
    let mut second = submit(&pipeline, chunk(&pipeline, 0, DATA_CHUNK_BYTES));
    let before = pipeline.stats().charged_bytes;
    assert_eq!(second.cancel(), DataCancelOutcome::Cancelled);
    assert!(pipeline.stats().charged_bytes <= before - DATA_CHUNK_BYTES);
    assert!(matches!(
        second.wait(deadline()),
        Err(DataPipelineError::Cancelled)
    ));
    assert_eq!(pipeline.stats().cancelled, 1);
    harness.gate.release();
    ok(&mut first);
    until(|| pipeline.stats().outstanding == 0);
    assert_eq!(harness.events.lock().unwrap().len(), 1);
}

#[test]
fn expired_queue_entry_is_not_published_and_wait_does_not_cancel() {
    let harness = Harness::new(Some(data_op::OPEN_WRITER));
    let pipeline = harness.pipeline(1, 4);
    let mut first = submit(&pipeline, open(&pipeline));
    harness.entered();
    let mut second = pipeline
        .submit(
            &open(&pipeline),
            Instant::now() + Duration::from_millis(60),
            DataAdmission::Reject,
        )
        .unwrap();
    assert_eq!(
        second.wait(Instant::now()),
        Err(DataPipelineError::WaitTimeout)
    );
    std::thread::sleep(Duration::from_millis(90));
    harness.gate.release();
    ok(&mut first);
    assert!(matches!(
        second.wait(deadline()),
        Err(DataPipelineError::Deadline {
            may_have_been_sent: false
        })
    ));
    assert_eq!(harness.events.lock().unwrap().len(), 1);
}

#[test]
fn wait_admission_requires_result_consumption_and_obeys_deadline() {
    let harness = Harness::new(Some(data_op::OPEN_WRITER));
    let pipeline = harness.pipeline(1, 1);
    let mut first = submit(&pipeline, open(&pipeline));
    harness.entered();
    let start = Instant::now();
    assert!(matches!(
        pipeline.submit(
            &open(&pipeline),
            start + Duration::from_millis(50),
            DataAdmission::Wait
        ),
        Err(DataPipelineError::Deadline {
            may_have_been_sent: false
        })
    ));
    assert!(start.elapsed() < Duration::from_secs(1));
    std::thread::scope(|scope| {
        let waiting = scope.spawn(|| {
            pipeline
                .submit(&open(&pipeline), deadline(), DataAdmission::Wait)
                .unwrap()
        });
        harness.gate.release();
        until(|| pipeline.stats().finished == 1);
        assert!(!waiting.is_finished());
        ok(&mut first);
        let mut second = waiting.join().unwrap();
        ok(&mut second);
    });
    until(|| pipeline.stats().outstanding == 0);
}

#[test]
fn incoming_capacity_and_result_memory_enforce_byte_budget() {
    let harness = Harness::new(Some(data_op::OPEN_WRITER));
    let pipeline = harness
        .client
        .draw_data_pipeline(
            DataPipelineConfig {
                parallel_requests: 1,
                max_operations: 8,
                max_bytes: (2 << 20) + (96 << 10),
                ..Default::default()
            },
            deadline(),
        )
        .unwrap();
    let mut first = submit(&pipeline, open(&pipeline));
    harness.entered();
    let mut oversized = open(&pipeline);
    oversized.payload = Vec::with_capacity(4 << 20);
    let start = Instant::now();
    assert!(matches!(
        pipeline.submit(&oversized, deadline(), DataAdmission::Wait),
        Err(DataPipelineError::Full)
    ));
    assert!(start.elapsed() < Duration::from_millis(100));
    let mut tickets = Vec::new();
    loop {
        match pipeline.submit(
            &chunk(&pipeline, 0, DATA_CHUNK_BYTES),
            deadline(),
            DataAdmission::Reject,
        ) {
            Ok(ticket) => tickets.push(ticket),
            Err(DataPipelineError::Full) => break,
            Err(error) => panic!("{error}"),
        }
    }
    assert!(pipeline.stats().outstanding < 8);
    assert!(pipeline.stats().peak_bytes <= (2 << 20) + (96 << 10));
    for ticket in &mut tickets {
        ticket.cancel();
        assert!(matches!(
            ticket.wait(deadline()),
            Err(DataPipelineError::Cancelled)
        ));
    }
    harness.gate.release();
    ok(&mut first);
    until(|| pipeline.stats().outstanding == 0);
}

#[test]
fn dropping_a_ticket_does_not_silently_drop_a_mutation() {
    let harness = Harness::new(Some(data_op::OPEN_WRITER));
    let pipeline = harness.pipeline(1, 2);
    let mut first = submit(&pipeline, open(&pipeline));
    harness.entered();
    drop(submit(&pipeline, open(&pipeline)));
    harness.gate.release();
    ok(&mut first);
    until(|| pipeline.stats().outstanding == 0);
    assert_eq!(harness.events.lock().unwrap().len(), 2);
}

#[test]
fn close_cancels_queued_work_and_joins_active_calls() {
    let harness = Harness::new(Some(data_op::OPEN_WRITER));
    let pipeline = harness
        .client
        .draw_data_pipeline(
            DataPipelineConfig {
                parallel_requests: 1,
                max_operations: 3,
                request_timeout: Duration::from_millis(80),
                ..Default::default()
            },
            deadline(),
        )
        .unwrap();
    let mut first = submit(&pipeline, open(&pipeline));
    harness.entered();
    let mut second = submit(&pipeline, open(&pipeline));
    pipeline.close();
    assert!(matches!(
        first.wait(deadline()),
        Err(DataPipelineError::Deadline {
            may_have_been_sent: true
        })
    ));
    assert!(matches!(
        second.wait(deadline()),
        Err(DataPipelineError::Cancelled)
    ));
    assert_eq!(
        (
            pipeline.stats().queued,
            pipeline.stats().running,
            pipeline.stats().outstanding
        ),
        (0, 0, 0)
    );
    assert!(matches!(
        pipeline.submit(&open(&pipeline), deadline(), DataAdmission::Reject),
        Err(DataPipelineError::Closed)
    ));
}

#[test]
fn invalid_config_epoch_and_operations_do_not_enter_queue() {
    let harness = Harness::new(None);
    for config in [
        DataPipelineConfig {
            parallel_requests: 0,
            ..Default::default()
        },
        DataPipelineConfig {
            max_operations: 1025,
            ..Default::default()
        },
        DataPipelineConfig {
            max_bytes: 1024,
            ..Default::default()
        },
        DataPipelineConfig {
            request_timeout: Duration::from_secs(61),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            harness.client.draw_data_pipeline(config, deadline()),
            Err(DataPipelineError::Invalid(_))
        ));
    }
    let pipeline = harness.pipeline(1, 2);
    let mut request = open(&pipeline);
    request.host_epoch += 1;
    assert!(matches!(
        pipeline.submit(&request, deadline(), DataAdmission::Reject),
        Err(DataPipelineError::Invalid(_))
    ));
    let mut request = open(&pipeline);
    request.flags = 1;
    assert!(matches!(
        pipeline.submit(&request, deadline(), DataAdmission::Reject),
        Err(DataPipelineError::Invalid(_))
    ));
    assert_eq!(pipeline.stats().submitted, 0);
    assert!(harness.events.lock().unwrap().is_empty());
}

#[test]
fn admitted_metadata_is_validated_on_a_fixed_worker_before_publication() {
    let harness = Harness::new(None);
    let pipeline = harness.pipeline(1, 2);
    let invalid = DataRequest {
        host_epoch: pipeline.host_epoch(),
        metadata: b"not json".to_vec(),
        ..DataRequest::new(data_op::LIST)
    };
    let mut ticket = submit(&pipeline, invalid);
    assert!(matches!(
        ticket.wait(deadline()),
        Err(DataPipelineError::Protocol(_))
    ));
    assert!(harness.events.lock().unwrap().is_empty());
    assert_eq!(pipeline.stats().submitted, 1);
    ok(&mut submit(&pipeline, open(&pipeline)));
}

#[test]
fn concurrent_producers_share_one_budget_without_dropping_requests() {
    let harness = Harness::new(None);
    let pipeline = harness.pipeline(4, 8);
    let baseline = pipeline.stats().charged_bytes;
    std::thread::scope(|scope| {
        for producer in 0..8 {
            let pipeline = &pipeline;
            scope.spawn(move || {
                for sequence in 0..64 {
                    let request = chunk(pipeline, (producer * 64 + sequence) * 32, 32);
                    let mut ticket = pipeline
                        .submit(&request, deadline(), DataAdmission::Wait)
                        .unwrap();
                    ok(&mut ticket);
                }
            });
        }
    });
    pipeline.close();
    let stats = pipeline.stats();
    assert_eq!(
        (stats.submitted, stats.finished, stats.outstanding),
        (512, 512, 0)
    );
    assert_eq!(stats.charged_bytes, baseline);
    assert!(stats.peak_bytes <= 32 << 20);
    let events = harness.events.lock().unwrap();
    assert_eq!(events.len(), 512);
    assert_eq!(
        events
            .iter()
            .map(|event| event.1)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        512
    );
}

#[test]
fn shutdown_wakes_capacity_waiters_without_consuming_their_input() {
    let harness = Harness::new(None);
    let pipeline = harness.pipeline(1, 1);
    let mut first = submit(&pipeline, open(&pipeline));
    until(|| pipeline.stats().finished == 1);
    std::thread::scope(|scope| {
        let input = open(&pipeline);
        let pipeline = &pipeline;
        let waiting = scope.spawn(move || {
            let result = pipeline.submit(&input, deadline(), DataAdmission::Wait);
            assert_eq!(input.operation, data_op::OPEN_WRITER);
            assert!(matches!(result, Err(DataPipelineError::Closed)));
        });
        pipeline.shutdown();
        waiting.join().unwrap();
    });
    ok(&mut first);
    pipeline.close();
    assert_eq!(pipeline.stats().outstanding, 0);
    assert_eq!(harness.events.lock().unwrap().len(), 1);
}

#[test]
fn pipelined_pages_commit_atomically_through_the_native_retained_service() {
    let host = gearbox_api::fake::FakeHost::start("pipelined-storage", &[]).unwrap();
    let client =
        Client::connect_id(host.endpoint_id, IdentitySource::Random, "pipelined-upload").unwrap();
    let pipeline = client
        .draw_data_pipeline(
            DataPipelineConfig {
                parallel_requests: 4,
                max_operations: 8,
                ..Default::default()
            },
            deadline(),
        )
        .unwrap();
    let baseline = pipeline.stats().charged_bytes;
    let fixture = upload::UploadFixture::cloud("pipeline-live", "cloud", 100_000).unwrap();
    let mut connection = upload::Connection::Pipeline {
        client: &pipeline,
        window: 8,
    };
    let lease = connection.open().unwrap();
    let first = connection.upload(&fixture, lease, 1, None).unwrap();
    connection.verify(&fixture, first).unwrap();
    let second = connection.upload(&fixture, lease, 2, Some(first)).unwrap();
    assert_eq!(second.incarnation, first.incarnation);
    assert_eq!(second.generation, first.generation + 1);
    connection.verify(&fixture, second).unwrap();
    connection.clear("pipeline-live", lease).unwrap();
    connection.close_writer(lease).unwrap();
    pipeline.close();
    assert_eq!(pipeline.stats().charged_bytes, baseline);
    assert_eq!(pipeline.stats().outstanding, 0);
    assert!(pipeline.stats().peak_bytes <= 32 << 20);
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
}

#[test]
fn borrowed_wire_uses_canonical_hash_and_decodes_after_admission() {
    let harness = Harness::new(None);
    let pipeline = harness.pipeline(1, 2);
    let env = open(&pipeline).pack_checked().unwrap();
    assert_eq!(env.type_hash(), 10_699_185_597_628_789_957);
    let mut ticket = pipeline
        .submit_wire(
            env.type_hash(),
            env.wire(),
            deadline(),
            DataAdmission::Reject,
        )
        .unwrap();
    ok(&mut ticket);
    let before = pipeline.stats().submitted;
    assert!(matches!(
        pipeline.submit_wire(
            peerbus::wire_type_hash::<DataRequest>(),
            env.wire(),
            deadline(),
            DataAdmission::Reject
        ),
        Err(DataPipelineError::Invalid(_))
    ));
    assert!(matches!(
        pipeline.submit_wire(
            env.type_hash(),
            &env.wire()[..16],
            deadline(),
            DataAdmission::Reject
        ),
        Err(DataPipelineError::Invalid(_))
    ));
    assert_eq!(pipeline.stats().submitted, before);
    let mut invalid = env.wire().to_vec();
    invalid[88..92].copy_from_slice(&999u32.to_le_bytes());
    let mut ticket = pipeline
        .submit_wire(env.type_hash(), &invalid, deadline(), DataAdmission::Reject)
        .unwrap();
    assert!(matches!(
        ticket.wait(deadline()),
        Err(DataPipelineError::Protocol(_))
    ));
    assert_eq!(harness.events.lock().unwrap().len(), 1);
    pipeline.close();
    assert_eq!(pipeline.stats().outstanding, 0);
}

#[test]
fn weak_cancellation_handle_interrupts_a_waiter_without_retaining_its_result() {
    let harness = Harness::new(Some(data_op::OPEN_WRITER));
    let pipeline = harness.pipeline(1, 2);
    let mut active = submit(&pipeline, open(&pipeline));
    harness.entered();
    let mut queued = submit(&pipeline, open(&pipeline));
    let cancel = queued.cancellation();
    std::thread::scope(|scope| {
        let waiter = scope.spawn(move || queued.wait(deadline()));
        assert_eq!(cancel.cancel(), DataCancelOutcome::Cancelled);
        assert_eq!(waiter.join().unwrap(), Err(DataPipelineError::Cancelled));
    });
    harness.gate.release();
    ok(&mut active);
    pipeline.close();
    assert_eq!(cancel.cancel(), DataCancelOutcome::Complete);
    assert_eq!(pipeline.stats().outstanding, 0);
}

#[test]
fn borrowed_wire_sections_share_admission_and_payload_accounting() {
    let harness = Harness::new(Some(data_op::OPEN_WRITER));
    let pipeline = harness.pipeline(1, 2);
    let baseline = pipeline.stats().charged_bytes;
    let mut active = submit(&pipeline, open(&pipeline));
    harness.entered();
    let request = chunk(&pipeline, 0, DATA_CHUNK_BYTES);
    let env = request.pack_checked().unwrap();
    let wire = env.wire();
    let parts = [&wire[..112], &wire[112..113], &wire[113..114], &wire[114..]];
    let before = pipeline.stats().charged_bytes;
    let mut queued = pipeline
        .submit_wire_parts(env.type_hash(), &parts, deadline(), DataAdmission::Reject)
        .unwrap();
    assert!(pipeline.stats().charged_bytes >= before + wire.len());
    let full = pipeline.stats().charged_bytes;
    assert!(matches!(
        pipeline.submit_wire_parts(env.type_hash(), &parts, deadline(), DataAdmission::Reject),
        Err(DataPipelineError::Full)
    ));
    assert_eq!(pipeline.stats().charged_bytes, full);
    for bad in [
        &parts[..0],
        &[&wire[..16]][..],
        &[wire, wire][..],
        &[wire; 5][..],
    ] {
        assert!(matches!(
            pipeline.submit_wire_parts(env.type_hash(), bad, deadline(), DataAdmission::Reject),
            Err(DataPipelineError::Invalid(_))
        ));
    }
    harness.gate.release();
    ok(&mut active);
    ok(&mut queued);
    pipeline.close();
    assert_eq!(pipeline.stats().charged_bytes, baseline);
    assert_eq!(pipeline.stats().outstanding, 0);
    assert_eq!(harness.events.lock().unwrap().len(), 2);
}
