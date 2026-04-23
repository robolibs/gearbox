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

use datapod::{Point, Size};

use crate::vehicle::{
    ChassisSpec, DriveMode, MeshSource, PartKind, PartSpec, PowerKind, PowerSource,
    VehicleBuilder, VehicleSpec, WheelSpec,
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

    let wheel_x = 1.50;
    let front_z = 0.75;
    let rear_z = -0.75;

    let target_bottom: f64 = -1.00;
    let conn_y = target_bottom + rest + radius;

    // Kingpin offset: the cylinder strut sits outboard of the tyre's
    // outer face (+X for right-side wheels, −X for left). Magnitude =
    // half-tyre-width + small visual gap. The wheel's visual hub
    // swings around this offset when steering.
    let kingpin_mag = width as f64 * 0.5 + 0.02;

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
        driven: true,
        steered: true,
        max_engine_force: 240.0,
        max_brake: 500.0,
        max_steer_rad: std::f64::consts::FRAC_PI_2,
        // Outboard kingpin: sign(x) times the offset magnitude.
        steering_pivot_offset: Point::new(x.signum() * kingpin_mag, 0.0, 0.0),
    };

    // --- Gantry side beams ------------------------------------------
    // Scale both side beams up by ~30% so the gantry reads heavier.
    let beam_h = 0.85_f64 * 1.30;
    // Old width before the 20 % inward expansion — kept so the beam's
    // OUTER face stays in its original lateral position (the extra
    // width grows inward only).
    let beam_w_old = 0.55_f64 * 1.30;
    let beam_w = beam_w_old * 1.20;
    // The outer face used to sit at `wheel_x + beam_w_old / 2`; keep
    // that outer face fixed and move the beam centre inboard so the
    // new width extends toward the gantry middle.
    let beam_outer_edge = wheel_x + beam_w_old * 0.5;
    let beam_centre_x = beam_outer_edge - beam_w * 0.5;
    // Raise the beam centre so its bottom sits comfortably above the
    // wheel tops even with the taller profile.
    let beam_y = 0.45_f64;
    // Beam length covers the full wheelbase plus a margin, then
    // extended by 40 % so the wheels sit entirely UNDER the beam
    // when viewed from above (no tyre poking out the front/back).
    let beam_len = ((front_z - rear_z) + 0.40) * 0.95 * 1.40;

    let side_beam_l = PartSpec {
        name: "side_beam_left".into(),
        position: Point::new(-beam_centre_x, beam_y, 0.0),
        size: Size::new(beam_w, beam_h, beam_len),
        color: robotti_red,
        kind: PartKind::Karosserie,
        mesh: MeshSource::Box,
    };
    let side_beam_r = PartSpec {
        name: "side_beam_right".into(),
        position: Point::new(beam_centre_x, beam_y, 0.0),
        size: Size::new(beam_w, beam_h, beam_len),
        color: robotti_red,
        kind: PartKind::Karosserie,
        mesh: MeshSource::Box,
    };

    // --- Front crossbar --------------------------------------------
    // Smaller than the side beams, with a slender rectangular section
    // so it reads as a lighter connector. Aligned at the BOTTOM of the
    // side beams — the crossbar's bottom face is flush with the beams'
    // bottom face, so it hangs at the lower edge of the gantry rather
    // than being centred. 20 % thinner in the two non-connection
    // directions (Y = height, Z = depth); the X length (the actual
    // "connection direction" spanning between the two side beams) is
    // unchanged.
    let cross_section = 0.22_f64 * 0.80;
    let cross_h = cross_section;
    let cross_thk = cross_section;
    let beam_bottom = beam_y - beam_h * 0.5;
    let cross_y = beam_bottom + cross_h * 0.5;
    let beam_front_edge_z = beam_len * 0.5;
    let cross_z = beam_front_edge_z - cross_thk * 0.5;
    // Span the inside-to-inside of the side beams at their new
    // (inward-shifted) position.
    let cross_len = (beam_centre_x * 2.0) - beam_w;
    let crossbar = PartSpec {
        name: "front_crossbar".into(),
        position: Point::new(0.0, cross_y, cross_z),
        size: Size::new(cross_len, cross_h, cross_thk),
        color: robotti_red,
        kind: PartKind::Karosserie,
        mesh: MeshSource::Box,
    };

    // --- Central pole + sensor box --------------------------------
    // A thin vertical CYLINDER passing through the middle of the
    // front crossbar (so pole and crossbar meet at 90° — "perpendicular
    // to the connector"), topped by a flat square sensor housing. The
    // pole itself tops out at the side-beam top so it doesn't stick
    // higher than the side structures; only the sensor box above is
    // what ends up above the beams.
    let beam_top = beam_y + beam_h * 0.5;

    let pole_dia = 0.09_f64; // 50% thinner than the previous pass (0.18)
    let pole_bottom = beam_bottom;
    let pole_top = beam_top; // flush with the side beams' tops
    let pole_h = pole_top - pole_bottom;
    let pole_y = (pole_top + pole_bottom) * 0.5;
    // Centre the pole on the crossbar in Z so the two cross at 90°.
    let pole_z = cross_z;
    let centre_pole = PartSpec {
        name: "centre_pole".into(),
        position: Point::new(0.0, pole_y, pole_z),
        size: Size::new(pole_dia, pole_h, pole_dia),
        color: robotti_red,
        kind: PartKind::Karosserie,
        mesh: MeshSource::Cylinder,
    };
    // Box sensor housing sitting on top of the pole — even flatter and
    // wider this pass (30 % wider on X, 50 % thinner on Y).
    let box_w = 0.40_f64 * 1.5 * 1.30; // 0.60 × 1.30 = 0.78
    let box_h = 0.25_f64 * 0.8 * 0.50; // 0.20 × 0.50 = 0.10
    let box_d = 0.40_f64;
    let sensor_box = PartSpec {
        name: "sensor_box".into(),
        position: Point::new(0.0, pole_top + box_h * 0.5, pole_z),
        size: Size::new(box_w, box_h, box_d),
        color: robotti_red,
        kind: PartKind::Karosserie,
        mesh: MeshSource::Box,
    };

    // --- King-pin struts (cylinders) -------------------------------
    // One per wheel. Positioned on the OUTER side of the wheel
    // (outboard by ~half the wheel width + small gap) so the strut
    // sits alongside the wheel rather than through its middle. The
    // strut drops from the side-beam bottom down to the wheel-hub
    // centre line, which is visually where the axle stub meets the
    // wheel on a real 4WIS assembly.
    let beam_bottom = beam_y - beam_h * 0.5;
    let wheel_hub_y = conn_y as f64 - rest as f64; // wheel centre at rest
    let strut_top = beam_bottom;
    let strut_bottom = wheel_hub_y; // stop at the hub centre line
    let strut_h = (strut_top - strut_bottom).max(0.05);
    let strut_centre_y = (strut_top + strut_bottom) * 0.5;
    let strut_dia = 0.12_f64;
    // Cylinder sits outboard of the tyre's outer face.
    let strut_outboard_x = wheel_x + width as f64 * 0.5 + 0.02;

    let make_strut = |name: &str, sign_x: f64, z: f64| PartSpec {
        name: name.into(),
        position: Point::new(sign_x * strut_outboard_x, strut_centre_y, z),
        size: Size::new(strut_dia, strut_h, strut_dia),
        color: [0.22, 0.22, 0.24],
        kind: PartKind::Hitch, // visual-only (avoid parry thin-AABB path)
        mesh: MeshSource::Cylinder,
    };

    VehicleBuilder::new("robotti", chassis)
        .max_speed(2.5)
        .wheel(make(wheel_x, front_z))
        .wheel(make(-wheel_x, front_z))
        .wheel(make(wheel_x, rear_z))
        .wheel(make(-wheel_x, rear_z))
        .part(side_beam_l)
        .part(side_beam_r)
        .part(crossbar)
        .part(centre_pole)
        .part(sensor_box)
        .part(make_strut("strut_fl", -1.0, front_z))
        .part(make_strut("strut_fr", 1.0, front_z))
        .part(make_strut("strut_rl", -1.0, rear_z))
        .part(make_strut("strut_rr", 1.0, rear_z))
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
