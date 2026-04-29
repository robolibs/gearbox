//! Oxbo 2475 pea harvester — port of
//! `flatsim_old/examples/machines/oxbo_harvester.json`.
//!
//! 6-wheeled articulated harvester: front wheels steer ±14°, middle
//! wheels are fixed, rear wheels steer **opposite** direction (±25°)
//! for crab-style turning. Rear-driven.

use datapod::{Point, Quaternion, Size};

use crate::vehicle::parts_lib;
use crate::vehicle::{
    ChassisSpec, Container, MeshSource, PartKind, PowerKind, PowerSource, VehicleBuilder,
    VehicleSpec, WheelSpec,
};

pub fn oxbo_harvester() -> VehicleSpec {
    // "Lower part" (chassis frame where the wheels are). 1.5 × 1.15 =
    // 1.725 m tall — 15 % up from the earlier 1.5 m per user feedback.
    let chassis_y = 1.725_f64;
    let chassis = ChassisSpec {
        size: Size::new(2.8, chassis_y, 7.68),
        mass: 15_000.0,
        com_offset: Point::new(0.0, -0.4, 0.0),
        linear_damping: 0.3,
        angular_damping: 2.5,
        ccd: false, // see tractor.rs — parry ray-AABB underflow on CCD
        color: [1.0, 0.784, 0.0], // flatsim (255, 200, 0)
        inertia_size: None,
        render_chassis: true,
        mesh: MeshSource::Box,
        usd_asset: None,
        usd_scene_offset: Point::new(0.0, 0.0, 0.0),
        usd_scene_rotation: Quaternion::identity(),
    };

    let radius = 0.768;
    let width = 0.7;

    let rest = 0.3;
    let stiffness = 160.0;
    let damping = 14.0;
    let friction = 28.0;
    let max_force = 60_000.0;

    // Wheels stick 0.35 m below the chassis bottom.
    let chassis_bottom = -chassis_y * 0.5;
    let target_bottom = chassis_bottom - 0.35;
    let conn_y = target_bottom + rest + radius;

    let front_steer =  14.0_f64.to_radians();
    let rear_steer  = -25.0_f64.to_radians(); // opposite-sign → crab turns

    let make = |x: f64,
                z: f64,
                driven: bool,
                engine: f64,
                steered: bool,
                max_steer: f64|
     -> WheelSpec {
        WheelSpec {
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
            driven,
            steered,
            max_engine_force: engine,
            max_brake: 2_500.0,
            max_steer_rad: max_steer,
            steering_pivot_offset: Point::new(0.0, 0.0, 0.0),
        usd_prim_path: None,
        usd_steer_prim_path: None,
        }
    };

    // Body parts — all in chassis yellow, matching the vehicle.
    //   HEAD big & wide, way out in front
    //   CAB  one uniform yellow box, shorter than before, poking
    //        slightly forward of the chassis
    //   BUNKER long yellow box filling the chassis top from right
    //        behind the cab all the way back. TOP Y = CAB TOP Y.
    //   Small rear engine panel.
    let yellow = chassis.color;
    let dark   = [0.22, 0.22, 0.24];

    let chassis_top: f64 = chassis_y * 0.5;
    let chassis_w:   f64 = 2.8;

    // Harvester head — big working implement jutting out the front.
    let head = parts_lib::cuboid(
        "harvest_head",
        Point::new(0.0, 0.0, 4.72),
        Size::new(3.53, 1.2, 1.77),
        yellow,
        PartKind::Karosserie,
    );

    // Chassis front / rear edges (Z extent of the lower part).
    let chassis_front_z: f64 =  7.68 * 0.5; //  3.84
    let chassis_rear_z:  f64 = -7.68 * 0.5; // -3.84

    // CAB — pokes forward of the chassis by a visible amount (0.45 m)
    // to make a clear front step. Full chassis width, 1.4 m tall.
    let cab_depth: f64  = 1.8;
    let cab_h: f64      = 1.40;
    let cab_front_z: f64 = chassis_front_z + 0.45;  // +4.29
    let cab_back_z:  f64 = cab_front_z - cab_depth; // +2.19
    let cab_center_z: f64 = (cab_front_z + cab_back_z) * 0.5;
    let cab = parts_lib::cab(cab_center_z, chassis_w, cab_h, cab_depth, chassis_top, yellow);
    // Thin dark roof cap.
    let cab_roof = parts_lib::cab_roof(
        cab_center_z,
        chassis_w,
        cab_depth,
        chassis_top + cab_h,
        0.14,
        0.10,
        dark,
    );

    // BUNKER — sticks out REARWARD of the chassis more than the cab
    // sticks out forward (1.0 m vs 0.45 m). Same height as cab.
    let bin_rear_z: f64   = chassis_rear_z - 1.00; // -4.84
    let bin_depth: f64    = cab_back_z - bin_rear_z;
    let bin_center_z: f64 = (cab_back_z + bin_rear_z) * 0.5;
    let bin_h: f64        = cab_h;
    let bunker = parts_lib::cuboid(
        "bunker",
        Point::new(0.0, chassis_top + bin_h * 0.5, bin_center_z),
        Size::new(chassis_w, bin_h, bin_depth),
        yellow,
        PartKind::Tank,
    );

    VehicleBuilder::new("oxbo_harvester", chassis)
        .max_speed(5.0)
        .wheel(make( 1.4,  2.304, false, 0.0,    true,  front_steer))
        .wheel(make(-1.4,  2.304, false, 0.0,    true,  front_steer))
        .wheel(make( 1.4,  0.100, false, 0.0,    false, 0.0))
        .wheel(make(-1.4,  0.100, false, 0.0,    false, 0.0))
        .wheel(make( 1.4, -2.688, true,  8000.0, true,  rear_steer))
        .wheel(make(-1.4, -2.688, true,  8000.0, true,  rear_steer))
        .part(head)
        .part(cab)
        .part(cab_roof)
        .part(bunker)
        // Harvester runs on diesel — big tank, heavy work drain when
        // the header's picking peas.
        .power_source(
            PowerSource::new(PowerKind::Fuel, "Fuel", 800.0)
                .with_travel_drain(2.0)
                .with_work_drain(5.0),
        )
        // Bunker — fills up while the header is engaged in work
        // mode and the vehicle is moving. Fill rate is expressed as a
        // fraction of capacity per second (2 % = 50 s to fill).
        .container(Container::new("Cont", 1500.0).with_fill_rate_frac(0.02))
        .build()
}
