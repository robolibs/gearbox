//! Tractor — dimensions ported from **John Deere 8R** real-world specs
//! (Deere spec sheet + Nebraska OECD Test 2141 for 8R 370/410):
//!
//!   overall length        5.99 m   (our chassis is 5.0 m — the real
//!                                   "overall" includes front weights
//!                                   and rear hitch linkage that
//!                                   stick out past the frame)
//!   overall width          2.55 m  (over fenders; chassis 2.00 m)
//!   height to cab top      3.33 m
//!   wheelbase              3.05 m  → front z = +1.525, rear z = -1.525
//!   front tyre 600/70R30  Ø 1.69 m, tread 0.60 m
//!   rear  tyre 710/70R42  Ø 2.05 m, tread 0.71 m
//!   shipping mass         ~12 000 kg (we use 8500 — matches the
//!                                     existing suspension tuning better)
//!   cab (CommandView III)  1.80 m long × 1.75 m tall × 1.75 m wide,
//!                          rear-aligned; ~30 % of overall length
//!   hood (engine cover)    ~2.80 m long, ~0.85 m tall; ~47 % of length

use datapod::{Point, Size};

use crate::vehicle::{ChassisSpec, PartKind, PartSpec, VehicleBuilder, VehicleSpec, WheelSpec};

const MAX_STEER_RAD: f32 = 0.6109; // 35°

pub fn tractor() -> VehicleSpec {
    let chassis_x = 2.00_f64;
    let chassis_y = 1.30_f64;
    let chassis_z = 5.00_f64;

    let chassis = ChassisSpec {
        size: Size::new(chassis_x, chassis_y, chassis_z),
        mass: 8500.0,
        // Lower COM — big top-heavy cab makes roll-overs easy without it.
        com_offset: Point::new(0.0, -0.35, 0.0),
        linear_damping: 0.2,
        angular_damping: 2.0,
        ccd: true,
        color: [0.0, 1.0, 0.392], // John Deere green
    };

    // Suspension tuning.
    let rest = 0.35;
    let stiffness = 180.0;
    let damping = 16.0;
    let friction = 26.0;
    let max_force = 60_000.0;

    // Real tyre dimensions.
    let front_radius = 0.845; // 1.69 m / 2
    let front_width  = 0.60;
    let rear_radius  = 1.025; // 2.05 m / 2
    let rear_width   = 0.71;

    // Wheels stick 0.30 m below the chassis bottom for the suspension
    // raycast to have room.
    let chassis_bottom = -chassis_y as f32 * 0.5;
    let target_bottom  = chassis_bottom - 0.30;
    let front_conn_y   = target_bottom + rest + front_radius;
    let rear_conn_y    = target_bottom + rest + rear_radius;

    // Wheelbase 3.05 m → front at +1.525, rear at -1.525.
    // Lateral x = ±1.15 so the tyres poke slightly outboard of the
    // chassis (chassis half-width is 1.00 m) — matches the real look.
    let wheel_x = 1.15;
    let front_z = 1.525;
    let rear_z  = -1.525;

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
        max_brake: 15.0,
        max_steer_rad: MAX_STEER_RAD,
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
        max_engine_force: 10_000.0,
        max_brake: 40.0,
        max_steer_rad: 0.0,
    };

    // ─── Body parts ─────────────────────────────────────────────────
    //   HOOD in front (lower, engine cover shape)
    //   CAB in back (full chassis height again, rear-aligned)
    //   ROOF dark cap on cab
    //   REAR HITCH marker
    let body_green = chassis.color;
    let yellow     = [0.95, 0.85, 0.15];

    let chassis_top:  f64 = chassis_y * 0.5;
    let chassis_back: f64 = -chassis_z * 0.5;
    let chassis_front: f64 = chassis_z * 0.5;

    // CAB — 1.75 m tall, 3.04 m long (2.34 × 1.30, another 30 %
    // forward extension on top of the previous 30 %). Rear still
    // aligned to the chassis back, full chassis width.
    let cab_h: f64       = 1.75;
    let cab_depth: f64   = 3.04;
    let cab_center_z: f64 = chassis_back + cab_depth * 0.5; // -0.98
    let cab = PartSpec {
        name: "cab".into(),
        position: Point::new(0.0, chassis_top + cab_h * 0.5, cab_center_z),
        size: Size::new(chassis_x, cab_h, cab_depth),
        color: body_green,
        kind: PartKind::Karosserie,
    };
    // Thin dark roof, slight overhang.
    let roof = PartSpec {
        name: "roof".into(),
        position: Point::new(0.0, chassis_top + cab_h + 0.08, cab_center_z),
        size: Size::new(chassis_x + 0.12, 0.14, cab_depth + 0.12),
        color: [0.22, 0.22, 0.24],
        kind: PartKind::Karosserie,
    };
    // HOOD — meets the cab flush (no gap). Runs from the cab's front
    // face to the same forward position it occupied before.
    let cab_front_z: f64 = cab_center_z + cab_depth * 0.5;     // -0.16
    let hood_h: f64      = 0.85;
    let hood_back_z: f64 = cab_front_z;                        // -0.16 — flush
    let hood_front_z: f64 = chassis_front - 0.35;              // +2.15
    let hood_depth: f64   = hood_front_z - hood_back_z;        //  2.31
    let hood_center_z: f64 = (hood_back_z + hood_front_z) * 0.5;
    let hood = PartSpec {
        name: "hood".into(),
        position: Point::new(0.0, chassis_top + hood_h * 0.5, hood_center_z),
        size: Size::new(chassis_x, hood_h, hood_depth),
        color: body_green,
        kind: PartKind::Karosserie,
    };
    // Rear hitch marker — small cube behind the cab.
    let rear_hitch = PartSpec {
        name: "rear_hitch".into(),
        position: Point::new(0.0, -0.30, chassis_back - 0.10),
        size: Size::new(0.20, 0.20, 0.20),
        color: yellow,
        kind: PartKind::Hitch,
    };
    // Front weights — small dark block at the nose.
    let weights = PartSpec {
        name: "front_weights".into(),
        position: Point::new(0.0, -0.20, chassis_front + 0.15),
        size: Size::new(1.20, 0.70, 0.30),
        color: [0.25, 0.25, 0.28],
        kind: PartKind::Karosserie,
    };

    VehicleBuilder::new("tractor", chassis)
        .wheel(front(wheel_x))
        .wheel(front(-wheel_x))
        .wheel(rear(wheel_x))
        .wheel(rear(-wheel_x))
        .part(hood)
        .part(cab)
        .part(roof)
        .part(weights)
        .part(rear_hitch)
        .build()
}
