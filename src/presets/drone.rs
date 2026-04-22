//! Quad-copter drone — arcade flight model.
//!
//! Not a ground vehicle: no wheels, no suspension. `DriveMode::Drone`
//! tells the sim to apply direct forces/torques to the chassis rigid
//! body instead of feeding a wheel controller. The body is a small
//! central slab with four rotor arms sticking out the corners.
//!
//! Controls (wired in `viz::input::wasd_input_system`):
//!   W / S   — fly forward / backward
//!   A / D   — strafe left / right
//!   Q / E   — yaw left / right
//!   Z / X   — ascend / descend

use datapod::{Point, Size};

use crate::vehicle::{
    ChassisSpec, DriveMode, PartKind, PartShape, PartSpec, VehicleBuilder, VehicleSpec,
};

pub fn drone() -> VehicleSpec {
    // --- Central slab ----------------------------------------------
    // Small quad-copter, ~35 cm across, 1 kg. Moderate angular
    // damping keeps the yaw from spinning forever once the user lets
    // go of Q/E; `linear_damping` is zero so the drone coasts
    // naturally in air.
    let chassis_x = 0.30_f64;
    let chassis_y = 0.08_f64;
    let chassis_z = 0.30_f64;
    let chassis = ChassisSpec {
        size: Size::new(chassis_x, chassis_y, chassis_z),
        mass: 1.0,
        com_offset: Point::new(0.0, 0.0, 0.0),
        // Mild linear damping keeps the drone from drifting forever
        // after a burst of thrust; heavy angular damping self-levels
        // the chassis quickly once the pitch/roll/yaw commands go
        // to zero (it's the arcade stand-in for a real PID
        // attitude-stabiliser).
        linear_damping: 0.4,
        angular_damping: 6.0,
        ccd: false, // see tractor.rs — parry ray-AABB underflow on CCD
        // Bright saturated orange — picked so the selection accent
        // driven from this colour stays colourful instead of
        // collapsing to a grey tint that'd wash out the UI fonts.
        color: [1.0, 0.52, 0.08],
        inertia_size: None,
        render_chassis: true,
    };

    // --- Rotor arms & blades ---------------------------------------
    // Four arms extend diagonally from the body's corners; a small
    // flat disc at the tip reads as a spinning rotor. Visual-only —
    // the flight model applies forces at the CoM, not per-rotor.
    let arm_len = 0.18;   // from body corner out to rotor hub
    let arm_h   = 0.03;
    let arm_w   = 0.04;
    let rotor_r = 0.10;   // rotor disc radius (size.x == size.z)

    let body_half_x = (chassis_x as f32) * 0.5;
    let body_half_z = (chassis_z as f32) * 0.5;
    let diag_off = arm_len * std::f32::consts::FRAC_1_SQRT_2;
    let rotor_y = (chassis_y as f32) * 0.5 + 0.02;

    // Arms stay a dark neutral so the orange body pops; rotor discs
    // are kept bright but intentionally a different hue from the
    // chassis so the props read clearly while spinning.
    let arm_color   = [0.08, 0.08, 0.10];
    let rotor_color = [0.90, 0.90, 0.92];

    let make_arm = |name: &str, dir_x: f32, dir_z: f32| PartSpec {
        name: name.into(),
        position: Point::new(
            (body_half_x * dir_x.signum() + diag_off * dir_x * 0.5) as f64,
            0.0,
            (body_half_z * dir_z.signum() + diag_off * dir_z * 0.5) as f64,
        ),
        size: Size::new(arm_w as f64, arm_h as f64, arm_len as f64),
        color: arm_color,
        // `Hitch` → visual-only: no rapier collider is built for this
        // part, only its Bevy mesh. Necessary for the drone because:
        //   * the arms are thin (0.04 × 0.03 × 0.18 m),
        //   * the rotor discs are *extremely* thin (0.20 × 0.01 × 0.20 m),
        // and parry 0.26's broad-phase raycast / BVH traversal hits a
        // `ray_aabb.rs:60` underflow when a zero-direction ray ends
        // up starting inside one of those razor-thin AABBs. Dropping
        // the colliders side-steps the bug entirely without affecting
        // what the drone looks like.
        kind: PartKind::Hitch,
        shape: PartShape::Box,
    };
    let make_rotor = |name: &str, dir_x: f32, dir_z: f32| PartSpec {
        name: name.into(),
        position: Point::new(
            (dir_x * (body_half_x + diag_off)) as f64,
            rotor_y as f64,
            (dir_z * (body_half_z + diag_off)) as f64,
        ),
        size: Size::new((rotor_r * 2.0) as f64, 0.01, (rotor_r * 2.0) as f64),
        color: rotor_color,
        kind: PartKind::Hitch, // visual-only (see note above)
        shape: PartShape::Box,
    };

    VehicleBuilder::new("drone", chassis)
        // Four diagonal arms.
        .part(make_arm("arm_fr",  1.0,  1.0))
        .part(make_arm("arm_fl", -1.0,  1.0))
        .part(make_arm("arm_rr",  1.0, -1.0))
        .part(make_arm("arm_rl", -1.0, -1.0))
        // Four rotor discs at the arm tips.
        .part(make_rotor("rotor_fr",  1.0,  1.0))
        .part(make_rotor("rotor_fl", -1.0,  1.0))
        .part(make_rotor("rotor_rr",  1.0, -1.0))
        .part(make_rotor("rotor_rl", -1.0, -1.0))
        .drive_mode(DriveMode::Drone)
        .build()
}
