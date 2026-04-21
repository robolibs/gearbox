//! Headless smoke test: prove the sim runs with no Bevy in scope.
//!
//! Spawn a tractor above the ground, step it for a second of simulated time,
//! and assert it has settled onto the ground without exploding or tunneling.
//!
//! Run with:
//!     cargo test --no-default-features --test headless

use gearbox::{
    datapod::{Point, Pose, Quaternion},
    presets, ControlInput, Sim,
};

#[test]
fn tractor_settles_and_drives() {
    let mut sim = Sim::new();
    sim.add_ground_plane(100.0);

    let spec = presets::tractor();
    // Spawn just above the rest height so settling has minimal impact
    // energy and the chassis doesn't bounce-pitch off the ground.
    let start_y = 1.4;
    let id = sim.spawn_vehicle(
        spec,
        Pose { point: Point::new(0.0, start_y, 0.0), rotation: Quaternion::identity() },
    );

    let dt = 1.0 / 60.0;
    for _ in 0..120 {
        sim.step(dt);
    }
    let settled = sim.vehicle_pose(id).point;
    assert!(
        settled.y > 0.0 && settled.y < start_y,
        "tractor should have fallen from {start_y} to somewhere above the ground; got y={}",
        settled.y
    );

    sim.set_control(
        id,
        ControlInput { throttle: 1.0, brake: 0.0, steer: 0.0, yaw: 0.0, lift: 0.0 },
    );
    for _ in 0..180 {
        sim.step(dt);
    }
    let driven = sim.vehicle_pose(id).point;
    let dx = driven.x - settled.x;
    let dy = driven.y - settled.y;
    let dz = driven.z - settled.z;
    let travelled = (dx * dx + dy * dy + dz * dz).sqrt();
    assert!(
        travelled > 1.0,
        "tractor should have moved > 1 m under throttle; travelled {travelled:.3} m"
    );
}
