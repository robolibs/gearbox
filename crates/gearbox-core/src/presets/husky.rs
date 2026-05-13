//! Clearpath Husky — differential-drive outdoor robot. Dimensions
//! ported directly from the flatsim URDF:
//!   `/home/bresilla/data/code/ares/flatsim/machines/urdf/husky.urdf`
//!
//! Body 0.67 × 0.30 × 0.99 m (x, y, z in gearbox — flatsim uses
//! x-lateral, y-longitudinal), four equal wheels r = 0.125 m,
//! w = 0.15 m, mounted at (±0.335, -, ±0.30). No steering joints —
//! turning is pure skid-steer via [`DriveMode::Differential`].

use datapod::{Point, Quaternion, Size};

use crate::vehicle::{
    ChassisSpec, DriveMode, MeshSource, PowerKind, PowerSource, VehicleBuilder, VehicleSpec,
    WheelSpec,
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
        // CCD off — see tractor.rs for the parry underflow rationale.
        ccd: false,
        // Light lavender — lifted further from the original deep
        // purple so the body reads pastel / luminous against the
        // sandy ground without looking washed-out.
        color: [0.82, 0.55, 1.00],
        inertia_size: None,
        render_chassis: false,
        mesh: MeshSource::Box,
        usd_asset: Some("machines/husky.usdc"),
        // Husky URDF→USD: X = forward, Y = lateral (left), Z = up.
        // bevy_openusd's `rot_x(-π/2)` flip alone leaves USD-X mapped
        // to bevy-X (= gearbox lateral), which puts the husky's
        // forward axis sideways. Compose `rot_y(-π/2)` on top so the
        // net (`rot_y(-π/2) * rot_x(-π/2)`) maps USD-X → bevy-Z
        // (forward), USD-Y → bevy-X (lateral), USD-Z → bevy-Y (up).
        //
        // Lower the SceneRoot until the USD wheel-bottoms (= USD-Z
        // -0.132 + offset) touch the rapier raycast wheel-bottom
        // plane. With raycast wheels carrying the chassis, the
        // settled chassis Y ≈ wheel_radius + chassis_y/2 + clearance,
        // and we want USD origin → settled chassis Y - 0.132.
        usd_scene_offset: Point::new(0.0, -0.10, 0.0),
        usd_scene_rotation: Quaternion::new(
            std::f64::consts::FRAC_1_SQRT_2,
            0.0,
            -std::f64::consts::FRAC_1_SQRT_2,
            0.0,
        ),
    };

    // --- Suspension + wheels ---------------------------------------
    // Radius bumped from 0.125 → 0.165 to match the husky USD's
    // authored wheel mesh (USDC encodes wheel-centre 0.165 m above
    // wheel-bottom). With the matching radius the rapier raycast
    // and USD wheel meshes overlap exactly when settled.
    let radius = 0.165;
    let width = 0.15;

    let rest = 0.06;
    let stiffness = 20.0;
    let damping = 2.5;
    let friction = 22.0;
    let max_force = 4_000.0;

    // Wheels hang 22 cm below the chassis bottom — about half the
    // wheel radius sticks out under the body, so ground clearance
    // ends up near the top of the wheel circumference. Keeps the
    // underside well clear of the terrain.
    let chassis_bottom = -chassis_y * 0.5;
    let target_bottom = chassis_bottom - 0.22;
    let conn_y = target_bottom + rest + radius;

    // Wheel positions taken straight from the husky USD prim
    // hierarchy after the `rot_y(-π/2) * rot_x(-π/2)` orientation
    // fix: USD-Y (lateral) → gearbox `+X`, USD-X (forward) → gearbox
    // `+Z`. So wheel-x = ±0.285 and front-z = ±0.256.
    let wheel_x = 0.285;
    let front_z = 0.256;
    let rear_z = -0.256;

    let make = |x: f64, z: f64, prim: &'static str| WheelSpec {
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
        driven: true,   // all four wheels driven on a skid-steer
        steered: false, // no steering joints — `Differential` mode ignores this
        // Linear + angular both scale with `max_engine_force`, so
        // cutting it by 4× again (12.5 → 3.125 N per wheel) slows
        // both the straight-line dash and the spin-in-place together.
        // Keeps the steer/throttle ratio set by `TURN_GAIN = 6.0`
        // intact — motion just happens at a calmer pace.
        max_engine_force: 3.125,
        max_brake: 2.5,
        max_steer_rad: 0.0,
        steering_pivot_offset: Point::new(0.0, 0.0, 0.0),
        usd_prim_path: Some(prim),
        usd_steer_prim_path: None,
    };

    VehicleBuilder::new("husky", chassis)
        .max_speed(1.5)
        .wheel(make(
            wheel_x,
            front_z,
            "/husky/base_link/front_left_wheel_link",
        ))
        .wheel(make(
            -wheel_x,
            front_z,
            "/husky/base_link/front_right_wheel_link",
        ))
        .wheel(make(
            wheel_x,
            rear_z,
            "/husky/base_link/rear_left_wheel_link",
        ))
        .wheel(make(
            -wheel_x,
            rear_z,
            "/husky/base_link/rear_right_wheel_link",
        ))
        // No `.part(...)` — the USD scene supplies the visible body.
        .drive_mode(DriveMode::Differential)
        .power_source(
            PowerSource::new(PowerKind::Battery, "Battery", 200.0)
                .with_travel_drain(0.8)
                .with_work_drain(0.4),
        )
        .build()
}
