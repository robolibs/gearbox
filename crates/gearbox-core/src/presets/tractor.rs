//! Tractor — small utility tractor, sized to match the flatsim
//! tractor URDF rather than the John-Deere-8R scale we used before.
//! Flatsim body is 1.6 × 2.8 m with r=0.42 m wheels; we keep the same
//! overall footprint but preserve the JD-style visual silhouette
//! (small front / big rear wheels, cab + hood + weights).
//!
//! Reference: `/home/bresilla/data/code/ares/flatsim/machines/urdf/tractor.urdf`.

use datapod::{Point, Size};

use crate::vehicle::parts_lib;
use crate::vehicle::{
    ChassisSpec, MeshSource, PartKind, PowerKind, PowerSource, VehicleBuilder, VehicleSpec,
    WheelSpec,
};

const MAX_STEER_RAD: f64 = 0.6109; // 35°

/// Uniform scale applied to every length, mass, force and torque in
/// this preset. Keeping lengths × `SCALE`, mass × `SCALE³` and forces
/// × `SCALE³` keeps the handling feel (accel, suspension compression,
/// brake distance) unchanged while just making the tractor bigger.
const SCALE: f64 = 1.15;

pub fn tractor() -> VehicleSpec {
    let s = SCALE;
    let s3 = s * s * s;

    let chassis_x = 1.40_f64 * SCALE;
    // Chassis box (the "lower" section where the wheels attach) made
    // 30 % taller — 0.80 m → 1.04 m — so the frame reads as a proper
    // utility-tractor base instead of a low slab. The cab rides on
    // top unchanged, so the whole silhouette lifts with it.
    let chassis_y = 1.04_f64 * SCALE;
    let chassis_z = 2.80_f64 * SCALE;

    let chassis = ChassisSpec {
        size: Size::new(chassis_x, chassis_y, chassis_z),
        mass: 2500.0 * s3,
        com_offset: Point::new(0.0, -0.22 * SCALE, 0.0),
        linear_damping: 0.2,
        angular_damping: 2.0,
        // CCD disabled — rapier/parry 0.26 has a broad-phase ray
        // bug (`ray_aabb.rs:60` underflow when a zero-dir ray
        // starts inside an AABB) that the CCD path reliably
        // triggers on startup. Vehicle speeds here are way below
        // the tunnelling threshold anyway.
        ccd: false,
        color: [0.0, 1.0, 0.392], // John Deere green
        inertia_size: None,
        render_chassis: true,
        mesh: MeshSource::Box,
    };

    // Suspension — stiffness/damping scaled down with the mass so the
    // visibly-bouncy feel from the larger preset carries over.
    let rest = 0.22 * s;
    let stiffness = 55.0 * s3;
    let damping = 4.5 * s3;
    let friction = 24.0;
    let max_force = 18_000.0 * s3;

    // Tyre dimensions — JD-style front-small/rear-big distinction,
    // trimmed back 10 % from the larger stance. Front tyre 40 %
    // thinner than the rear so the nose reads as steered rather than
    // matching the drive axle.
    let front_radius = 0.378 * s;
    let front_width  = 0.227 * s;
    let rear_radius  = 0.594 * s;
    let rear_width   = 0.432 * s;

    // Wheels stick 30 cm below the chassis bottom — the previous
    // 20 cm left the frame sitting very low; this raises the whole
    // tractor a touch for a stance closer to the JD reference.
    let chassis_bottom = -chassis_y * 0.5;
    let target_bottom  = chassis_bottom - 0.30 * s;
    let front_conn_y   = target_bottom + rest + front_radius;
    let rear_conn_y    = target_bottom + rest + rear_radius;

    // Wheelbase 1.70 m → front at +0.85, rear at -0.85.
    // Lateral x = ±0.75 so tyres poke slightly outboard of the
    // chassis (chassis half-width is 0.70 m).
    let wheel_x = 0.75 * SCALE;
    let front_z = 0.85 * SCALE;
    let rear_z  = -0.85 * SCALE;

    let front = |x: f64| WheelSpec {
        chassis_connection: Point::new(x, front_conn_y as f64, front_z),
        suspension_dir: Point::new(0.0, -1.0, 0.0),
        axle_dir: Point::new(-1.0, 0.0, 0.0),
        suspension_rest_length: rest,
        suspension_stiffness: stiffness,
        suspension_damping: damping,
        max_suspension_force: max_force,
        friction_slip: friction,
        radius: front_radius,
        width: front_width,
        driven: false,
        steered: true,
        max_engine_force: 0.0,
        max_brake: 600.0 * s3,
        max_steer_rad: MAX_STEER_RAD,
        steering_pivot_offset: Point::new(0.0, 0.0, 0.0),
    };
    let rear = |x: f64| WheelSpec {
        chassis_connection: Point::new(x, rear_conn_y as f64, rear_z),
        suspension_dir: Point::new(0.0, -1.0, 0.0),
        axle_dir: Point::new(-1.0, 0.0, 0.0),
        suspension_rest_length: rest,
        suspension_stiffness: stiffness,
        suspension_damping: damping,
        max_suspension_force: max_force,
        friction_slip: friction,
        radius: rear_radius,
        width: rear_width,
        driven: true,
        steered: false,
        max_engine_force: 4_000.0 * s3,
        max_brake: 1_500.0 * s3,
        max_steer_rad: 0.0,
        steering_pivot_offset: Point::new(0.0, 0.0, 0.0),
    };

    // ─── Body parts ─────────────────────────────────────────────────
    //   CAB — the only superstructure; hood removed.
    //   ROOF dark cap on cab
    //   REAR HITCH marker
    //   FRONT WEIGHTS small dark block at the nose
    let body_green = chassis.color;
    let yellow     = [0.95, 0.85, 0.15];

    let chassis_top:   f64 = chassis_y * 0.5;
    let chassis_back:  f64 = -chassis_z * 0.5;
    let chassis_front: f64 =  chassis_z * 0.5;

    // CAB — 0.88 m tall, 1.59 m long. Rear edge sits 7.5 cm forward
    // of the chassis back (was 15 cm — halved for a smaller
    // drawbar-notch).
    let cab_h: f64       = 0.88 * SCALE;
    let cab_depth: f64   = 1.59 * SCALE;
    let cab_center_z: f64 = chassis_back + cab_depth * 0.5 + 0.075 * SCALE;
    let cab = parts_lib::cab(cab_center_z, chassis_x, cab_h, cab_depth, chassis_top, body_green);
    // Thin dark roof, slight overhang.
    let roof = parts_lib::cab_roof(
        cab_center_z,
        chassis_x,
        cab_depth,
        chassis_top + cab_h,
        0.09 * SCALE,
        0.08 * SCALE,
        [0.22, 0.22, 0.24],
    );
    // Rear hitch marker — small cube behind the cab.
    let rear_hitch = parts_lib::hitch_marker(
        "rear_hitch",
        Point::new(0.0, -0.18 * SCALE, chassis_back - 0.06 * SCALE),
        0.12 * SCALE,
        yellow,
    );
    // Front weights — small dark block at the nose.
    let weights = parts_lib::cuboid(
        "front_weights",
        Point::new(0.0, -0.12 * SCALE, chassis_front + 0.09 * SCALE),
        Size::new(0.70 * SCALE, 0.40 * SCALE, 0.18 * SCALE),
        [0.25, 0.25, 0.28],
        PartKind::Karosserie,
    );

    VehicleBuilder::new("tractor", chassis)
        .wheel(front(wheel_x))
        .wheel(front(-wheel_x))
        .wheel(rear(wheel_x))
        .wheel(rear(-wheel_x))
        .part(cab)
        .part(roof)
        .part(weights)
        .part(rear_hitch)
        // Diesel tank — travel drain moderate, work (PTO, implement)
        // is where the real consumption happens.
        .power_source(
            PowerSource::new(PowerKind::Fuel, "Fuel", 300.0)
                .with_travel_drain(1.2)
                .with_work_drain(2.5),
        )
        .build()
}
