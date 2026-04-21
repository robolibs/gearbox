//! Clearpath Husky — differential-drive outdoor robot. Dimensions
//! ported directly from the flatsim URDF:
//!   `/home/bresilla/data/code/ares/flatsim/machines/urdf/husky.urdf`
//!
//! Body 0.67 × 0.30 × 0.99 m (x, y, z in gearbox — flatsim uses
//! x-lateral, y-longitudinal), four equal wheels r = 0.125 m,
//! w = 0.15 m, mounted at (±0.335, -, ±0.30). No steering joints —
//! turning is pure skid-steer via [`DriveMode::Differential`].

use datapod::{Point, Size};

use crate::vehicle::{
    ChassisSpec, DriveMode, PartKind, PartSpec, VehicleBuilder, VehicleSpec, WheelSpec,
};

pub fn husky() -> VehicleSpec {
    // --- Chassis ----------------------------------------------------
    // Back to the flatsim body height (0.30 m). The extra 12 cm I'd
    // added made the whole box look too thick — raising the robot
    // above the ground is handled by the wheel-protrusion offset
    // below, not by fattening the chassis.
    let chassis_x = 0.67_f64;
    let chassis_y = 0.30_f64;
    let chassis_z = 0.99_f64;

    let chassis = ChassisSpec {
        size: Size::new(chassis_x, chassis_y, chassis_z),
        mass: 50.0,
        com_offset: Point::new(0.0, -0.05, 0.0),
        linear_damping: 0.2,
        angular_damping: 2.5,
        ccd: true,
        // Brightened from flatsim's deep purple — more luminous /
        // saturated so the robot reads against the sandy ground.
        color: [0.68, 0.30, 1.00],
    };

    // --- Suspension + wheels ---------------------------------------
    let radius = 0.125;
    let width  = 0.15;

    let rest = 0.06;
    let stiffness = 20.0;
    let damping = 2.5;
    let friction = 22.0;
    let max_force = 4_000.0;

    // Wheels hang 22 cm below the chassis bottom — about half the
    // wheel radius sticks out under the body, so ground clearance
    // ends up near the top of the wheel circumference. Keeps the
    // underside well clear of the terrain.
    let chassis_bottom = -chassis_y as f32 * 0.5;
    let target_bottom  = chassis_bottom - 0.22;
    let conn_y = target_bottom + rest + radius;

    // Flatsim husky wheel positions: (±0.335, ±0.30) in the
    // x-lateral / y-longitudinal convention of the URDF; swap to
    // gearbox (x-lateral / z-longitudinal).
    let wheel_x = 0.335;
    let front_z =  0.30;
    let rear_z  = -0.30;

    let make = |x: f64, z: f64| WheelSpec {
        chassis_connection: Point::new(x, conn_y as f64, z),
        suspension_dir: Point::new(0.0, -1.0, 0.0),
        axle_dir: Point::new(-1.0, 0.0, 0.0),
        suspension_rest_length: rest,
        suspension_stiffness: stiffness,
        suspension_damping: damping,
        max_suspension_force: max_force,
        friction_slip: friction,
        radius,
        width,
        driven: true,  // all four wheels driven on a skid-steer
        steered: false, // no steering joints — `Differential` mode ignores this
        // Real Husky tops out at ~1 m/s. Cut another 4× from the
        // previous 50 N setting — 12.5 N × 4 wheels on a 50 kg body
        // gives ~1 m/s² acceleration, which reads as a slow, careful
        // robot instead of a rocket.
        max_engine_force: 12.5,
        max_brake: 10.0,
        max_steer_rad: 0.0,
    };

    // --- Sensor / battery marker parts ------------------------------
    // Battery sits inside the base; keep it as a visual-only marker
    // (Hitch kind → no collider, just a small dark block).
    let chassis_top: f64 = chassis_y * 0.5;
    let battery = PartSpec {
        name: "battery".into(),
        position: Point::new(0.0, -0.02, 0.0),
        size: Size::new(0.30, 0.15, 0.20),
        color: [0.10, 0.10, 0.12],
        kind: PartKind::Hitch, // visual-only
    };
    // A small raised "plate" on top for sensor mounts — purely
    // aesthetic; keeps the silhouette recognisable as a Husky.
    let top_plate = PartSpec {
        name: "top_plate".into(),
        position: Point::new(0.0, chassis_top + 0.02, 0.0),
        size: Size::new(chassis_x * 0.9, 0.04, chassis_z * 0.7),
        color: [0.30, 0.30, 0.34],
        kind: PartKind::Karosserie,
    };

    VehicleBuilder::new("husky", chassis)
        .wheel(make( wheel_x, front_z))
        .wheel(make(-wheel_x, front_z))
        .wheel(make( wheel_x, rear_z))
        .wheel(make(-wheel_x, rear_z))
        .part(top_plate)
        .part(battery)
        .drive_mode(DriveMode::Differential)
        .build()
}
