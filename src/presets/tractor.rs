//! Tractor — port of `flatsim_old/examples/machines/tractor.json`
//! (John Deere 8R).
//!
//! Coordinate map from flatsim's 2-D top-down JSON:
//!   flatsim.x (lateral)      → gearbox.x
//!   flatsim.y (longitudinal) → gearbox.z  (positive = forward)
//!   wheel `size.width`       → gearbox `wheel.width`  (tyre thickness)
//!   wheel `size.height`      → gearbox `wheel.radius * 2`
//!
//! Flatsim is 2-D so there is no vertical (Y) chassis size — we pick
//! 1.4 m, typical for a row-crop tractor.

use datapod::{Point, Size};

use crate::vehicle::{ChassisSpec, PartKind, PartSpec, VehicleBuilder, VehicleSpec, WheelSpec};

const MAX_STEER_RAD: f32 = 0.6109; // 35°

pub fn tractor() -> VehicleSpec {
    let chassis_y = 1.4_f64;
    let chassis = ChassisSpec {
        size: Size::new(1.6, chassis_y, 2.8),
        mass: 3000.0,
        com_offset: Point::new(0.0, -0.3, 0.0),
        linear_damping: 0.2,
        angular_damping: 1.5,
        ccd: true,
        color: [0.0, 1.0, 0.392], // flatsim (0, 255, 100)
    };

    // Suspension tuning.
    let rest = 0.3;
    let stiffness = 100.0;
    let damping = 10.0;
    let friction = 25.0;
    let max_force = 30_000.0;

    let front_radius = 0.42;
    let front_width  = 0.352;
    let rear_radius  = 0.70;
    let rear_width   = 0.64;

    // Wheels stick 0.3 m BELOW the chassis bottom so there's room for
    // the suspension raycast to find the ground. Same convention as
    // every other preset in this folder.
    let chassis_bottom = -chassis_y as f32 * 0.5;
    let target_bottom = chassis_bottom - 0.3;
    let front_conn_y = target_bottom + rest + front_radius;
    let rear_conn_y  = target_bottom + rest + rear_radius;

    let front = |x: f64| WheelSpec {
        chassis_connection: Point::new(x, front_conn_y as f64, 0.84),
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
        max_brake: 10.0,
        max_steer_rad: MAX_STEER_RAD,
    };
    let rear = |x: f64| WheelSpec {
        chassis_connection: Point::new(x, rear_conn_y as f64, -0.84),
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
        max_engine_force: 5000.0,
        max_brake: 30.0,
        max_steer_rad: 0.0,
    };

    // Body parts — ONE cab that matches the chassis footprint (same
    // width AND same length), chassis colour. Thin dark roof cap.
    // Small rear hitch marker.
    let body_green = chassis.color;
    let yellow     = [0.95, 0.85, 0.15];

    let chassis_top: f64 = chassis_y * 0.5;
    let chassis_w:   f64 = 1.6;
    let chassis_z:   f64 = 2.8;

    // Cab — 60 % of chassis HEIGHT and 60 % of chassis LENGTH, full
    // chassis width, REAR-aligned (cab back = chassis back).
    let cab_h: f64      = chassis_y * 0.60;          // 0.84
    let cab_depth: f64  = chassis_z * 0.60;          // 1.68
    let chassis_back_z: f64 = -chassis_z * 0.5;      // -1.40
    let cab_center_z: f64 = chassis_back_z + cab_depth * 0.5;  // -0.56
    let cab = PartSpec {
        name: "cab".into(),
        position: Point::new(0.0, chassis_top + cab_h * 0.5, cab_center_z),
        size: Size::new(chassis_w, cab_h, cab_depth),
        color: body_green,
        kind: PartKind::Karosserie,
    };
    // Thin dark roof cap, slight overhang.
    let roof = PartSpec {
        name: "roof".into(),
        position: Point::new(0.0, chassis_top + cab_h + 0.07, cab_center_z),
        size: Size::new(chassis_w + 0.10, 0.14, cab_depth + 0.10),
        color: [0.22, 0.22, 0.24],
        kind: PartKind::Karosserie,
    };
    // Rear hitch marker.
    let rear_hitch = PartSpec {
        name: "rear_hitch".into(),
        position: Point::new(0.0, -0.45, -1.52),
        size: Size::new(0.12, 0.12, 0.12),
        color: yellow,
        kind: PartKind::Hitch,
    };

    VehicleBuilder::new("tractor", chassis)
        .wheel(front(0.8))
        .wheel(front(-0.8))
        .wheel(rear(0.8))
        .wheel(rear(-0.8))
        .part(cab)
        .part(roof)
        .part(rear_hitch)
        .build()
}
