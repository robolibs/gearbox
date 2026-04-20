use datapod::{Point, Size};

use crate::vehicle::{ChassisSpec, VehicleBuilder, VehicleSpec, WheelSpec};

const MAX_STEER_RAD: f32 = 0.61; // ~35°

/// Rear-wheel-drive tractor: tall chassis, big driven rear wheels,
/// small steered front wheels.
pub fn tractor() -> VehicleSpec {
    let chassis = ChassisSpec {
        size: Size::new(1.8, 1.4, 3.6),
        mass: 3000.0,
        com_offset: Point::new(0.0, -0.3, 0.0),
        linear_damping: 0.2,
        angular_damping: 1.5,
        ccd: true,
    };

    // Rapier's canonical tuning (see `vehicle_controller3` example).
    // friction_slip in [10, 30] gives tractor-tyre grip; lower values feel
    // like driving on ice.
    let rest = 0.3;
    let stiffness = 100.0;
    let damping = 10.0;
    let friction = 25.0;

    let front_radius = 0.35;
    let rear_radius = 0.6;

    // Place the wheel ATTACHMENT heights so that at rest, both sets have
    // their bottoms at the same world Y. Otherwise the chassis settles
    // pitched and wheels on the "tall" end lose ground contact.
    // wheel_bottom = conn_y - rest_length - radius.  Target = -1.0.
    let front_conn_y = -1.0 + rest + front_radius; // -0.35
    let rear_conn_y  = -1.0 + rest + rear_radius;  // -0.10

    // Each wheel must carry ~7500 N (3000 kg / 4 wheels · g). Bump the
    // suspension force ceiling well above that so rapier doesn't cap it.
    let max_force = 30_000.0;

    let front = |x: f64| WheelSpec {
        chassis_connection: Point::new(x, front_conn_y as f64, 1.2),
        suspension_dir: Point::new(0.0, -1.0, 0.0),
        axle_dir: Point::new(-1.0, 0.0, 0.0),
        suspension_rest_length: rest,
        suspension_stiffness: stiffness,
        suspension_damping: damping,
        max_suspension_force: max_force,
        friction_slip: friction,
        radius: front_radius,
        width: 0.25,
        driven: false,
        steered: true,
        max_engine_force: 0.0,
        max_brake: 10.0,
        max_steer_rad: MAX_STEER_RAD,
    };

    let rear = |x: f64| WheelSpec {
        chassis_connection: Point::new(x, rear_conn_y as f64, -1.0),
        suspension_dir: Point::new(0.0, -1.0, 0.0),
        axle_dir: Point::new(-1.0, 0.0, 0.0),
        suspension_rest_length: rest,
        suspension_stiffness: stiffness,
        suspension_damping: damping,
        max_suspension_force: max_force,
        friction_slip: friction,
        radius: rear_radius,
        width: 0.35,
        driven: true,
        steered: false,
        max_engine_force: 5000.0,
        max_brake: 30.0,
        max_steer_rad: 0.0,
    };

    VehicleBuilder::new("tractor", chassis)
        .wheel(front(0.8))
        .wheel(front(-0.8))
        .wheel(rear(0.95))
        .wheel(rear(-0.95))
        .build()
}
