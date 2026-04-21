use datapod::{Point, Size};

use crate::vehicle::{ChassisSpec, VehicleBuilder, VehicleSpec, WheelSpec};

const MAX_STEER_RAD: f32 = 0.52; // ~30°

/// Small rear-wheel-drive car preset.
///
/// 4 equal wheels, front-steered, rear-driven.
pub fn car() -> VehicleSpec {
    let chassis = ChassisSpec {
        size: Size::new(1.8, 0.8, 4.0),
        mass: 1400.0,
        com_offset: Point::new(0.0, -0.2, 0.0),
        linear_damping: 0.1,
        angular_damping: 0.7,
        ccd: true,
        color: [0.82, 0.15, 0.15], // deep red
    };

    // Rapier's canonical tuning — see tractor.rs for the rationale.
    let radius = 0.34;
    let rest = 0.25;
    let stiffness = 80.0;
    let damping = 8.0;
    let friction = 20.0;
    let wheel_y = -0.2;

    // 1400 kg / 4 wheels · g ≈ 3500 N per wheel.
    let max_force = 15_000.0;

    let front = |x: f64| WheelSpec {
        chassis_connection: Point::new(x, wheel_y, 1.5),
        suspension_dir: Point::new(0.0, -1.0, 0.0),
        axle_dir: Point::new(-1.0, 0.0, 0.0),
        suspension_rest_length: rest,
        suspension_stiffness: stiffness,
        suspension_damping: damping,
        max_suspension_force: max_force,
        friction_slip: friction,
        radius,
        width: 0.22,
        driven: false,
        steered: true,
        max_engine_force: 0.0,
        max_brake: 1_200.0,
        max_steer_rad: MAX_STEER_RAD,
    };

    let rear = |x: f64| WheelSpec {
        chassis_connection: Point::new(x, wheel_y, -1.5),
        suspension_dir: Point::new(0.0, -1.0, 0.0),
        axle_dir: Point::new(-1.0, 0.0, 0.0),
        suspension_rest_length: rest,
        suspension_stiffness: stiffness,
        suspension_damping: damping,
        max_suspension_force: max_force,
        friction_slip: friction,
        radius,
        width: 0.22,
        driven: true,
        steered: false,
        max_engine_force: 5000.0,
        max_brake: 1_800.0,
        max_steer_rad: 0.0,
    };

    VehicleBuilder::new("car", chassis)
        .wheel(front(0.85))
        .wheel(front(-0.85))
        .wheel(rear(0.85))
        .wheel(rear(-0.85))
        .build()
}
