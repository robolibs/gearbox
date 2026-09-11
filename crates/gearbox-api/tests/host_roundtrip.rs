use std::time::Duration;

use gearbox_api::fake::FakeHost;
use gearbox_api::{Client, IdentitySource, UsdLoad, clock_op, code, next_sample, object_kind};

fn client(host: &FakeHost, name: &str) -> Client {
    Client::connect_id(host.endpoint_id, IdentitySource::Random, name).expect("client agent")
}

#[test]
fn host_info_load_list_and_events() {
    let host = FakeHost::start("test-host", &[]).expect("fake host");
    let c = client(&host, "t1");
    let info = c
        .wait_ready(Duration::from_secs(10))
        .expect("host answers info");
    assert_eq!(info.props().get("name").as_deref(), Some("fake"));

    let mut events = c.events().expect("event subscriber");
    let status = c
        .load(&UsdLoad::new("bale_1", "markers/bale.usdz").at(1.0, 0.0, 2.0))
        .expect("load call");
    assert!(status.is_ok(), "{}", status.message());

    let objects = c.list(object_kind::ANY).expect("list");
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].props().get("id").as_deref(), Some("bale_1"));

    let ev = next_sample::<gearbox_api::SceneEvent>(&mut events, Duration::from_secs(5))
        .expect("event recv")
        .expect("one loaded event");
    assert_eq!(ev.id(), "bale_1");
    assert_eq!(ev.x, 1.0);

    let clock = c.clock(clock_op::PAUSE).expect("clock");
    assert_eq!(clock.paused, 1);
    let status = c.clear(0).expect("clear");
    assert!(status.is_ok());
    assert!(c.list(object_kind::ANY).expect("list").is_empty());
}

#[test]
fn machine_claim_drive_and_busy() {
    let host = FakeHost::start("test-machines", &["oxbo"]).expect("fake host");
    let a = client(&host, "driver-a");
    a.wait_ready(Duration::from_secs(10))
        .expect("host answers info");

    let machines = a.machines().expect("machines list");
    assert_eq!(machines.len(), 1);
    assert_eq!(machines[0].namespace(), "oxbo");
    assert!(machines[0].did().starts_with("did:key:"));

    let mut oxbo = a.machine("oxbo");
    let info = oxbo.info().expect("machine info");
    assert_eq!(
        info.controllers_with_command("cmd_vel"),
        vec!["drive".to_string()]
    );

    let claim = oxbo.claim(500, false).expect("claim");
    assert_eq!(claim.code, code::OK);
    let session = claim.session;
    assert!(session > 0);

    let mut state = oxbo.state().expect("state subscriber");
    for _ in 0..10 {
        let status = oxbo.cmd_vel(session, 1.0, 0.0).expect("cmd_vel");
        assert!(status.is_ok(), "{}", status.message());
        std::thread::sleep(Duration::from_millis(50));
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let sample = loop {
        oxbo.cmd_vel(session, 1.0, 0.0).expect("cmd_vel");
        let sample = next_sample::<gearbox_api::MachineState>(&mut state, Duration::from_secs(5))
            .expect("state recv")
            .expect("state sample");
        let moving = sample.session == session && sample.linear_speed() > 0.0;
        if moving || std::time::Instant::now() > deadline {
            break sample;
        }
    };
    assert_eq!(sample.session, session);
    assert!(sample.linear_speed() > 0.0, "fake machine should be moving");

    let before = oxbo.session().expect("session info");
    assert_eq!(before.held, 1);
    assert_eq!(before.session, session);
    let b = client(&host, "driver-b");
    let refused = b.machine("oxbo").claim(500, false).expect("second claim");
    assert_eq!(refused.code, code::BUSY);
    assert!(!refused.holder().is_empty());

    let unsessioned = b.machine("oxbo").cmd_vel(0, 1.0, 0.0).expect("cmd_vel");
    assert_eq!(unsessioned.code, code::BUSY);

    assert!(oxbo.release(session).expect("release").is_ok());
    let taken = b
        .machine("oxbo")
        .claim(500, false)
        .expect("claim after release");
    assert_eq!(taken.code, code::OK);
}
