use std::sync::mpsc;
use std::time::{Duration, Instant};

use gearbox_api::{
    Client, ClockState, HostBus, HostConfig, HostInfo, IdentitySource, MachineAgent, MachineConfig, clock_op, code,
};

/// Serve `host` with `step` until `done` or 20 s pass.
fn pump(host: &mut HostBus, mut step: impl FnMut(&mut HostBus), mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() && Instant::now() < deadline {
        host.serve_info(|_| HostInfo::default());
        step(host);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn the_thread_answers_clock_reads_and_leaves_commands_for_the_frame() {
    let mut host = HostBus::new(HostConfig::new("test-clock-lane", "test").ephemeral()).expect("host");
    let lane = host.clock_lane().clone();
    let endpoint = host.endpoint_id();
    let (read_tx, read_rx) = mpsc::channel();
    let caller = std::thread::spawn(move || {
        let c = Client::connect_id(endpoint, IdentitySource::Random, "clock-lane").expect("client");
        read_tx.send(c.clock(clock_op::GET).expect("read")).unwrap();
        c.clock(clock_op::PLAY).expect("play")
    });

    let mut read = None;
    pump(&mut host, |_| lane.answer_reads(42.0), || {
        read = read.take().or_else(|| read_rx.try_recv().ok());
        read.is_some()
    });
    let read = read.expect("the thread answered the read");
    assert_eq!((read.world_s, read.paused), (42.0, 0));

    let held = Instant::now() + Duration::from_secs(2);
    pump(&mut host, |_| lane.answer_reads(43.0), || caller.is_finished() || Instant::now() > held);
    assert!(!caller.is_finished(), "the thread answered a play");

    let paused = ClockState { paused: 1, ..Default::default() };
    let mut asked = Vec::new();
    pump(
        &mut host,
        |host| {
            host.serve_clock(paused.clone(), |cmd| {
                asked.push(cmd.op);
                ClockState { world_s: 7.0, ..Default::default() }
            });
        },
        || caller.is_finished(),
    );
    assert_eq!(asked, vec![clock_op::PLAY]);
    assert_eq!(caller.join().expect("caller").world_s, 7.0);
}

#[test]
fn a_machine_lane_takes_claims_and_twists_without_the_agent_s_poll() {
    let host = HostBus::new(HostConfig::new("test-machine-lane", "test").ephemeral()).expect("host");
    let mut config = MachineConfig::new("test-machine-lane", "rover").with_minimal_links();
    config.ephemeral = true;
    let agent = MachineAgent::new(config, host.endpoint_id()).expect("agent");
    let lane = agent.lane().clone();
    let (endpoint, did) = (host.endpoint_id(), agent.did());
    let caller = std::thread::spawn(move || {
        let c = Client::connect_id(endpoint, IdentitySource::Random, "machine-lane").expect("client");
        let mut rover = c.machine_by_did("rover", &did).expect("rover");
        let claim = rover.claim(60_000, false).expect("claim");
        assert_eq!(claim.code, code::OK);
        rover.cmd_vel(claim.session, 0.7, 0.2).expect("cmd_vel")
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !caller.is_finished() && Instant::now() < deadline {
        lane.serve();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(caller.join().expect("caller").is_ok());
    let twist = lane.twist();
    assert_eq!((twist.linear.vx, twist.angular.vz), (0.7, 0.2));
    assert_ne!(agent.session_id(), 0);
}
