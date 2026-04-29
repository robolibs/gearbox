//! AGROINTELLI Robotti — 4-wheel-independent-steer gantry field robot.
//!
//! Structure (all dimensions in metres, chassis-local frame):
//!   * Two **longitudinal side beams** — the two chunky rectangles that
//!     ride above the wheels on each side, running front-to-back.
//!   * One **front crossbar** — connects the two side beams only at the
//!     front; the rear is open so the robot can straddle a crop row.
//!   * Four **cylindrical king-pin struts** — one per wheel, mounted on
//!     the **outer** side of each wheel and dropping from the side
//!     beam down to the wheel-hub centre. The wheel then attaches to
//!     the strut at its outside face (where the axle would sit on a
//!     real vehicle).
//!   * A small hidden central chassis box is the physics rigid body;
//!     the visible gantry identity comes from the parts above. The
//!     chassis's inertia tensor is overridden so rapier treats the
//!     body as a full 3 m × 1 m × 2 m mass distribution (not a small
//!     central pod) — otherwise any horizontal wheel force pitches /
//!     rolls the chassis violently and the whole thing bounces.
//!
//! Drive: [`DriveMode::Omni`]. `W/S` moves forward/back, `A/D`
//! strafes, `Q/E` rotates in place. Steering is rate-limited in
//! `sim::apply_controls` so key presses don't snap wheels from 0° →
//! 90° in one tick (which used to shove the body sideways).

use datapod::{Point, Quaternion, Size};

use crate::vehicle::{
    ChassisSpec, DriveMode, MeshSource, PowerKind, PowerSource, VehicleBuilder, VehicleSpec,
    WheelSpec,
};

pub fn robotti() -> VehicleSpec {
    // --- Central chassis (physics body) -----------------------------
    // Small enclosure that sits at the geometric centre of the
    // gantry. Kept compact so the visible gantry frame reads as the
    // vehicle's silhouette; the tensor below broadcasts its real
    // 3 m × 1 m × 2 m extent to the physics solver.
    let chassis_x = 0.50_f64;
    let chassis_y = 0.25_f64;
    let chassis_z = 0.60_f64;

    // Outer footprint used for the inertia tensor so roll / pitch
    // behave like a 3 m-wide gantry, not a 0.5 m pod.
    let gantry_outer = Size::new(3.0, 1.0, 2.0);

    let robotti_red: [f32; 3] = [0.80, 0.10, 0.10];
    let chassis = ChassisSpec {
        size: Size::new(chassis_x, chassis_y, chassis_z),
        mass: 428.0,
        // Drop the CoM down near ground level so horizontal wheel
        // forces produce a small moment arm (tiny pitch/roll torque)
        // instead of levering the whole gantry up.
        com_offset: Point::new(0.0, -0.75, 0.0),
        linear_damping: 0.25,
        angular_damping: 4.0,
        ccd: false,
        color: robotti_red,
        inertia_size: Some(gantry_outer),
        // Don't draw the central chassis cuboid — the gantry frame
        // (side beams + crossbar + pole + sensor box) carries the
        // silhouette, and the chassis would otherwise float in the
        // middle of the frame looking wrong.
        render_chassis: false,
        mesh: MeshSource::Box,
        usd_asset: Some("machines/robotti.usdc"),
        // Drop the SceneRoot so USD wheels touch the ground —
        // measured floating ~5 cm with offset = -0.91.
        usd_scene_offset: Point::new(0.0, -0.96, 0.0),
        // Robotti's USD has X = forward (URDF convention). The
        // bevy_openusd Z↔Y flip leaves USD-X mapped to bevy-X, which
        // is gearbox *lateral*. Add `rot_y(-π/2)` so the asset's
        // forward (USD-X) lands on gearbox `+Z` (forward).
        usd_scene_rotation: Quaternion::new(
            std::f64::consts::FRAC_1_SQRT_2,
            0.0,
            -std::f64::consts::FRAC_1_SQRT_2,
            0.0,
        ),
    };

    // --- Wheels + suspension ----------------------------------------
    let radius = 0.40;
    let width = 0.20 * 1.30; // 30% thicker tyres

    let rest = 0.08;
    // Scaled for 428 kg / 4 wheels ≈ 107 kg per wheel. Slightly stiff
    // + well-damped so the body doesn't pogo when you press a key.
    let stiffness = 140.0;
    let damping = 14.0;
    let friction = 18.0;
    let max_force = 15_000.0;

    // Wheel positions taken from the robotti USD prim hierarchy
    // after the `rot_y(-π/2) * rot_x(-π/2)` orientation fix:
    // wheel-x = ±1.50, longitudinal z = ±0.775.
    let wheel_x = 1.50;
    let front_z = 0.775;
    let rear_z = -0.775;

    let target_bottom: f64 = -1.00;
    let conn_y = target_bottom + rest + radius;

    // Kingpin offset: the cylinder strut sits outboard of the tyre's
    // outer face (+X for right-side wheels, −X for left). Magnitude =
    // half-tyre-width + small visual gap. The wheel's visual hub
    // swings around this offset when steering.
    let kingpin_mag = width as f64 * 0.5 + 0.02;

    let make = |x: f64, z: f64, wheel_prim: &'static str, knuckle_prim: &'static str| WheelSpec {
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
        driven: true,
        steered: true,
        max_engine_force: 240.0,
        max_brake: 500.0,
        max_steer_rad: std::f64::consts::FRAC_PI_2,
        // Outboard kingpin: sign(x) times the offset magnitude.
        steering_pivot_offset: Point::new(x.signum() * kingpin_mag, 0.0, 0.0),
        usd_prim_path: Some(wheel_prim),
        // Robotti's URDF authors a SEPARATE steering knuckle prim
        // for each wheel — the wheel only spins around its axle, the
        // knuckle rotates around the kingpin axis. We tag both so
        // the visual matches: spin lands on `<knuckle>/<wheel>`,
        // steer lands on `<knuckle>`.
        usd_steer_prim_path: Some(knuckle_prim),
    };

    VehicleBuilder::new("robotti", chassis)
        .max_speed(2.5)
        .wheel(make( wheel_x, front_z,
            "/robotti/base_link/link_37/link_40", "/robotti/base_link/link_37")) // left front
        .wheel(make(-wheel_x, front_z,
            "/robotti/base_link/link_27/link_30", "/robotti/base_link/link_27")) // right front
        .wheel(make( wheel_x, rear_z,
            "/robotti/base_link/link_41/link_44", "/robotti/base_link/link_41")) // left rear
        .wheel(make(-wheel_x, rear_z,
            "/robotti/base_link/link_31/link_34", "/robotti/base_link/link_31")) // right rear
        // No `.part(...)` calls — the USD scene supplies the visible
        // gantry frame, struts, sensor box, etc.
        .drive_mode(DriveMode::Omni)
        // Hybrid power plant: battery for electrics / compute, fuel
        // for the main drivetrain. Both are drained independently;
        // EITHER running dry disables the vehicle.
        .power_source(
            PowerSource::new(PowerKind::Battery, "Battery", 150.0)
                .with_travel_drain(0.6)
                .with_work_drain(1.2),
        )
        .power_source(
            PowerSource::new(PowerKind::Fuel, "Fuel", 250.0)
                .with_travel_drain(1.0)
                .with_work_drain(2.0),
        )
        .build()
}
