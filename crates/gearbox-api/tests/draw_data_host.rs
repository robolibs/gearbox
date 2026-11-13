use std::time::{Duration, Instant};

use gearbox_api::agentio::{Agent, DirectoryMode, ExchangeKind};
use gearbox_api::fake::FakeHost;
use gearbox_api::host::{serve_req, shm_limits};
use gearbox_api::peerbus::EndpointId;
use gearbox_api::wire::{Env, draw_data::*, pack};
use gearbox_api::{
    Client, DataLane, DrawDataError, DrawEntity, HostBus, HostConfig, IdentitySource, Status,
    topics,
};

#[path = "support/draw_data_native.rs"]
mod native;

#[test]
fn native_retained_cloud_and_occupancy_share_the_legacy_registry_and_reset() {
    let host = FakeHost::start("data-retained-native", &[]).unwrap();
    let c = client(host.endpoint_id);
    let report = native::verify(&c, true).unwrap();
    assert_eq!(report["reset_verified"], true);
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

fn client(host: EndpointId) -> Client {
    Client::connect_id(host, IdentitySource::Random, "data-test-client").unwrap()
}

fn raw(client: &Client, host: EndpointId, topic: &str, env: &Env) -> DataReply {
    let mut transport = client
        .agent
        .by_id(host)
        .unwrap()
        .req_client::<Env, Env>(topic)
        .unwrap();
    let reply = transport.call(env).unwrap();
    DataReply::unpack_checked(reply.header().type_hash, reply.payload()).unwrap()
}

fn agent(name: &str) -> Agent {
    Agent::builder()
        .name(name)
        .identity(IdentitySource::Random)
        .directory(DirectoryMode::Replicated)
        .local_config(shm_limits())
        .no_relay()
        .build()
        .unwrap()
}

fn pump<T: Send + 'static>(
    lane: &DataLane,
    caps: &DataCapabilities,
    mut handler: impl FnMut(DataRequest, DataOperation) -> DataReply,
    caller: std::thread::JoinHandle<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !caller.is_finished() && Instant::now() < deadline {
        lane.serve(caps, &mut handler).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        caller.is_finished(),
        "native service did not finish within 20s"
    );
    caller.join().unwrap()
}

#[test]
fn full_host_registers_companion_routes_without_changing_ring_limits() {
    assert_eq!(
        (shm_limits().max_publishers, shm_limits().max_subscribers),
        (24, 16)
    );
    let host = HostBus::new(HostConfig::new("data-full-host", "test").ephemeral()).unwrap();
    for topic in [
        topics::DRAW_DATA_CAPABILITIES,
        topics::DRAW_DATA,
        topics::DRAW_DATA_LIST,
        topics::DRAW_DATA_STATUS,
    ] {
        let entry = host
            .agent
            .directory()
            .lookup_exchange(topic, ExchangeKind::ReqRes)
            .unwrap();
        assert_eq!(entry.endpoint_id(), host.endpoint_id());
    }
    assert_ne!(host.draw_data_lane().epoch(), 0);
}

#[test]
fn typed_negotiation_reports_a_silent_native_service_as_timeout() {
    let host = HostBus::new(HostConfig::new("data-silent-host", "test").ephemeral()).unwrap();
    let c = client(host.endpoint_id());
    assert!(matches!(
        c.draw_data_with_timeout(Duration::from_millis(100)),
        Err(DrawDataError::Timeout)
    ));
    assert!(matches!(
        c.draw_data_with_timeout(Duration::ZERO),
        Err(DrawDataError::Timeout)
    ));
    assert!(matches!(
        c.draw_data_with_timeout(Duration::MAX),
        Err(DrawDataError::Protocol(_))
    ));
}

#[test]
fn expired_typed_request_never_consumes_a_writer_lease() {
    let host = FakeHost::start("data-deadline-host", &[]).unwrap();
    let c = client(host.endpoint_id);
    let mut data = c.draw_data().unwrap();
    let mut open = DataRequest::new(data_op::OPEN_WRITER);
    open.host_epoch = data.capabilities().host_epoch;
    assert!(matches!(
        data.request_with_timeout(&open, Duration::ZERO),
        Err(DrawDataError::Timeout)
    ));
    assert!(matches!(
        data.request_with_deadline(&open, Instant::now() - Duration::from_millis(1)),
        Err(DrawDataError::Timeout)
    ));
    assert!(matches!(
        data.request_with_timeout(&open, Duration::MAX),
        Err(DrawDataError::Protocol(_))
    ));
    let mut leases = std::collections::HashSet::new();
    for _ in 0..data.capabilities().limits.writers {
        let reply = data.request(&open).unwrap();
        assert_eq!(reply.code, data_code::OK);
        assert_ne!(reply.lease, 0);
        assert!(leases.insert(reply.lease));
    }
    assert_eq!(data.request(&open).unwrap().code, data_code::BUDGET);
}

#[test]
fn fake_host_negotiates_truthfully_and_preserves_legacy_draw_lifecycle() {
    let host = FakeHost::start("data-fake-host", &[]).unwrap();
    let c = client(host.endpoint_id);
    let mut data = c.draw_data().unwrap();
    let caps = data.capabilities().clone();
    assert_eq!(caps.retained, [DatasetKind::Cloud, DatasetKind::Octree, DatasetKind::Surfels, DatasetKind::MeshBlocks, DatasetKind::Volume, DatasetKind::Elevation]);
    assert!(caps.rendered.is_empty());
    assert!(
        caps.features.atomic_generations
            && caps.features.delta_updates
            && caps.features.unified_listing
    );
    assert_eq!(caps.decodable, caps.retained);
    let mut open = DataRequest::new(data_op::OPEN_WRITER);
    open.host_epoch = caps.host_epoch;
    let reply = data.request(&open).unwrap();
    assert_eq!((reply.code, reply.stage), (data_code::OK, data_stage::NONE));
    assert_eq!((reply.stored_generation, reply.visible_generation), (0, 0));
    assert_ne!(reply.lease, 0);
    let mut list = DataRequest::new(data_op::LIST)
        .with_metadata(&DataListQuery::default())
        .unwrap();
    list.host_epoch = caps.host_epoch;
    assert_eq!(data.request(&list).unwrap().code, data_code::OK);
    let mut status = DataRequest::new(data_op::STATUS)
        .with_key(&DataKey::new("mapping", "cloud").unwrap())
        .unwrap()
        .with_metadata(&DataStatusQuery::default())
        .unwrap();
    status.host_epoch = caps.host_epoch;
    assert_eq!(data.request(&status).unwrap().code, data_code::NOT_FOUND);
    assert!(
        c.draw(&DrawEntity::points("compat", "points", &[[1.0, 2.0, 3.0]]))
            .unwrap()
            .is_ok()
    );
    assert_eq!(c.draw_list("compat").unwrap().len(), 1);
    assert!(c.draw_delete("compat", "points").unwrap().is_ok());
    assert!(c.draw_list("compat").unwrap().is_empty());
    assert!(
        c.draw(&DrawEntity::points("compat", "points", &[[0.0; 3]]))
            .unwrap()
            .is_ok()
    );
    assert!(c.draw_clear("compat").unwrap().is_ok());
    assert!(c.draw_list("compat").unwrap().is_empty());
}

#[test]
fn malformed_packets_wrong_routes_and_stale_epochs_receive_explicit_errors() {
    let host = FakeHost::start("data-invalid-host", &[]).unwrap();
    let c = client(host.endpoint_id);
    let epoch = c.draw_data_capabilities().unwrap().host_epoch;
    for env in [pack(&DrawEntity::clear("")), Env::new(0, vec![0; 100])] {
        let reply = raw(&c, host.endpoint_id, topics::DRAW_DATA, &env);
        assert_eq!(reply.code, data_code::INVALID);
        assert!(reply.message.len() <= 512);
    }
    let mut req = DataRequest::new(data_op::OPEN_WRITER);
    req.host_epoch = epoch;
    assert_eq!(
        raw(
            &c,
            host.endpoint_id,
            topics::DRAW_DATA_STATUS,
            &req.pack_checked().unwrap()
        )
        .code,
        data_code::INVALID
    );
    req.host_epoch ^= 1;
    if req.host_epoch == 0 {
        req.host_epoch = 2;
    }
    assert_eq!(
        raw(
            &c,
            host.endpoint_id,
            topics::DRAW_DATA,
            &req.pack_checked().unwrap()
        )
        .code,
        data_code::STALE_EPOCH
    );
    let mut req = DataRequest::new(data_op::LIST)
        .with_metadata(&serde_json::json!({"namespace":"", "limit":0,"cursor":null}))
        .unwrap();
    req.host_epoch = epoch;
    assert_eq!(
        raw(
            &c,
            host.endpoint_id,
            topics::DRAW_DATA_LIST,
            &req.pack_checked().unwrap()
        )
        .code,
        data_code::INVALID
    );
    req.flags = 1;
    assert_eq!(
        raw(&c, host.endpoint_id, topics::DRAW_DATA_LIST, &pack(&req)).code,
        data_code::INVALID
    );
    assert_eq!(c.draw_data_capabilities().unwrap().host_epoch, epoch);
}

#[test]
fn simultaneous_clients_share_the_same_epoch_and_keep_legacy_calls_working() {
    let host = FakeHost::start("data-many-clients", &[]).unwrap();
    let callers: Vec<_> = (0..3)
        .map(|i| {
            let endpoint = host.endpoint_id;
            std::thread::spawn(move || {
                let c = client(endpoint);
                let caps = c.draw_data_capabilities().unwrap();
                assert!(
                    c.draw(&DrawEntity::points(
                        "many",
                        &i.to_string(),
                        &[[i as f32; 3]]
                    ))
                    .unwrap()
                    .is_ok()
                );
                caps.host_epoch
            })
        })
        .collect();
    let epochs: Vec<_> = callers.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(epochs.iter().all(|e| *e == epochs[0]));
    let c = client(host.endpoint_id);
    assert_eq!(c.draw_list("many").unwrap().len(), 3);
}

#[test]
fn old_host_returns_unsupported_without_silently_converting_data_to_legacy() {
    let host = agent("data-old-host");
    let mut legacy = host.req_server::<Env, Env>(topics::DRAW).unwrap();
    let c = client(host.endpoint_id());
    assert!(matches!(
        c.draw_data_capabilities(),
        Err(DrawDataError::Unsupported(_))
    ));
    let caller = std::thread::spawn(move || {
        c.draw(&DrawEntity::points("old", "p", &[[0.0; 3]]))
            .unwrap()
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut received = 0;
    while !caller.is_finished() && Instant::now() < deadline {
        received += serve_req(&mut legacy, |_: DrawEntity| Status::ok());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(caller.is_finished());
    assert!(caller.join().unwrap().is_ok());
    assert_eq!(received, 1);
}

#[test]
fn host_epochs_change_even_when_the_logical_instance_name_is_reused() {
    let first = {
        let host = HostBus::new(HostConfig::new("data-restart", "test").ephemeral()).unwrap();
        host.draw_data_lane().epoch()
    };
    let host = HostBus::new(HostConfig::new("data-restart", "test").ephemeral()).unwrap();
    assert_ne!(first, host.draw_data_lane().epoch());
}

#[test]
fn invalid_handler_replies_are_replaced_with_bounded_internal_errors() {
    let host = agent("data-invalid-handler");
    let lane = DataLane::new(&host).unwrap();
    let caps = DataCapabilities::wire_only(lane.epoch());
    let endpoint = host.endpoint_id();
    let epoch = lane.epoch();
    let caller = std::thread::spawn(move || {
        let c = client(endpoint);
        let mut r = DataRequest::new(data_op::OPEN_WRITER);
        r.host_epoch = epoch;
        raw(&c, endpoint, topics::DRAW_DATA, &r.pack_checked().unwrap())
    });
    let reply = pump(&lane, &caps, |_, _| DataReply::default(), caller);
    assert_eq!(reply.code, data_code::INTERNAL);
    assert_eq!(reply.stage, data_stage::NONE);
}

#[test]
fn future_schema_without_overlap_is_unsupported_not_a_decode_failure() {
    let host = agent("data-future-schema");
    let lane = DataLane::new(&host).unwrap();
    let mut caps = DataCapabilities::wire_only(lane.epoch());
    caps.schemas = vec![2];
    let endpoint = host.endpoint_id();
    let caller = std::thread::spawn(move || {
        matches!(
            client(endpoint).draw_data_capabilities(),
            Err(DrawDataError::Unsupported(_))
        )
    });
    assert!(pump(
        &lane,
        &caps,
        |_, _| panic!("capabilities must not reach mutation handler"),
        caller
    ));
    caps.schemas = vec![1, 2];
    let caller =
        std::thread::spawn(move || client(endpoint).draw_data_capabilities().unwrap().schemas);
    assert_eq!(
        pump(&lane, &caps, |_, _| panic!("unexpected handler"), caller),
        [1, 2]
    );
}

#[test]
fn bare_clients_require_explicit_host_selection() {
    let c = Client::bare(IdentitySource::Random, "bare-data").unwrap();
    assert!(matches!(
        c.draw_data_capabilities(),
        Err(DrawDataError::Protocol(_))
    ));
    let host = FakeHost::start("data-bare-target", &[]).unwrap();
    assert!(c.draw_data_at(host.endpoint_id).is_ok());
}

#[test]
fn discovery_rejects_another_hosts_signed_topic_record() {
    let selected = agent("data-selected-host");
    let other = agent("data-other-host");
    let _lane = DataLane::new(&other).unwrap();
    let record = other
        .directory()
        .lookup_exchange(topics::DRAW_DATA_CAPABILITIES, ExchangeKind::ReqRes)
        .unwrap();
    selected.directory().register(record).unwrap();
    let c = client(selected.endpoint_id());
    assert!(
        matches!(c.draw_data_capabilities(), Err(DrawDataError::Protocol(message)) if message.contains("selected host"))
    );
}

#[test]
fn restart_with_the_same_endpoint_rejects_the_previous_instance_epoch() {
    let key = gearbox_api::agentio::resolve_identity(&IdentitySource::Random).unwrap();
    let build = || {
        Agent::builder()
            .name("data-same-endpoint")
            .identity(IdentitySource::Key(key.clone()))
            .directory(DirectoryMode::Replicated)
            .local_config(shm_limits())
            .no_relay()
            .build()
            .unwrap()
    };
    let (endpoint, old_epoch) = {
        let host = build();
        let lane = DataLane::new(&host).unwrap();
        (host.endpoint_id(), lane.epoch())
    };
    let host = build();
    assert_eq!(host.endpoint_id(), endpoint);
    let lane = DataLane::new(&host).unwrap();
    assert_ne!(lane.epoch(), old_epoch);
    let caps = DataCapabilities::wire_only(lane.epoch());
    let caller = std::thread::spawn(move || {
        let c = client(endpoint);
        let mut r = DataRequest::new(data_op::OPEN_WRITER);
        r.host_epoch = old_epoch;
        raw(&c, endpoint, topics::DRAW_DATA, &r.pack_checked().unwrap())
    });
    let reply = pump(
        &lane,
        &caps,
        |_, _| panic!("stale request reached storage"),
        caller,
    );
    assert_eq!(
        (reply.code, reply.host_epoch),
        (data_code::STALE_EPOCH, lane.epoch())
    );
}
