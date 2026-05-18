//! Headless smoke test: prove the sim runs with no Bevy in scope.
//!
//! Spawn a tractor above the ground, step it for a second of simulated time,
//! and assert it has settled onto the ground without exploding or tunneling.
//!
//! Run with:
//!     cargo test -p gearbox-physics --test headless

use gearbox_physics::{
    ControlInput, Sim,
    datapod::{Point, Pose, Quaternion},
    presets,
};

#[test]
fn tractor_settles_and_drives() {
    let mut sim = Sim::new();
    sim.add_ground_plane(100.0);

    let spec = presets::tractor_articulated();
    // Spawn just above the rest height so settling has minimal impact
    // energy and the chassis doesn't bounce-pitch off the ground.
    let start_y = 1.4;
    let id = sim.spawn_vehicle(
        spec,
        Pose {
            point: Point::new(0.0, start_y, 0.0),
            rotation: Quaternion::identity(),
        },
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
        ControlInput {
            throttle: 1.0,
            brake: 0.0,
            steer: 0.0,
            yaw: 0.0,
            lift: 0.0,
        },
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

/// The physical wheels must actually *rotate* under drive torque — the
/// raycast model's wheels never spun as rigid bodies. Spawn a tractor,
/// settle it, drive it, and assert the driven rear wheels have rolled.
#[test]
fn driven_wheels_spin_under_throttle() {
    let mut sim = Sim::new();
    sim.add_ground_plane(100.0);

    let id = sim.spawn_vehicle(
        presets::tractor_articulated(),
        Pose {
            point: Point::new(0.0, 1.4, 0.0),
            rotation: Quaternion::identity(),
        },
    );

    let dt = 1.0 / 60.0;
    for _ in 0..120 {
        sim.step(dt);
    }
    // Rear wheels (index 2, 3) are the driven pair on the articulated tractor.
    let rest_spin = sim.wheel_spin_angle(id, 2);

    sim.set_control(
        id,
        ControlInput {
            throttle: 1.0,
            brake: 0.0,
            steer: 0.0,
            yaw: 0.0,
            lift: 0.0,
        },
    );
    for _ in 0..120 {
        sim.step(dt);
    }
    let rolled = (sim.wheel_spin_angle(id, 2) - rest_spin).abs();
    assert!(
        rolled > 0.5,
        "driven rear wheel should have rolled under throttle; spin delta {rolled:.3} rad"
    );
}

/// Smoke test for the steered-wheel rig: the Robotti has four
/// independently-steered wheels, so its suspension joints are the
/// cylindrical (slide + kingpin-twist) variant. Spawn it, let it
/// settle, and assert it stays finite and near the origin — a
/// malformed steered joint would NaN or fling the body away.
#[test]
fn steered_vehicle_settles() {
    let mut sim = Sim::new();
    sim.add_ground_plane(100.0);

    let id = sim.spawn_vehicle(
        presets::robotti(),
        Pose {
            point: Point::new(0.0, 2.0, 0.0),
            rotation: Quaternion::identity(),
        },
    );

    let dt = 1.0 / 60.0;
    for _ in 0..180 {
        sim.step(dt);
    }

    let p = sim.vehicle_pose(id).point;
    assert!(
        p.x.is_finite() && p.y.is_finite() && p.z.is_finite(),
        "steered vehicle pose must stay finite; got ({}, {}, {})",
        p.x,
        p.y,
        p.z
    );
    assert!(
        p.y > 0.0 && p.y < 2.0,
        "robotti should settle onto the ground; y={}",
        p.y
    );
    assert!(
        p.x.abs() < 5.0 && p.z.abs() < 5.0,
        "robotti should stay near the origin while settling; x={} z={}",
        p.x,
        p.z
    );
}
