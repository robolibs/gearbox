//! Tractor matching `assets/machines/tractor_articulated.usda` (after
//! the 0.25× rescale). Same chassis layout the original `tractor`
//! preset uses, but with dimensions / wheel positions / wheel radii
//! taken straight from the USD scene so the raycast wheels line up
//! with the loaded mesh visuals.
//!
//! Wheel layout (USD authored with +Y = back, +Z = up — gearbox uses
//! +Z = forward, +Y = up, so we flip Y→Z and Z→Y on import):
//!
//! ```text
//!     USD                       gearbox-local
//!     ─────────────────────     ──────────────────────────
//!     BL  (+0.8475, +1.14,  +0.755)   (+0.8475, ?, -1.14)
//!     BR  (-0.8475, +1.14,  +0.755)   (-0.8475, ?, -1.14)
//!     FL  (+0.79,   -1.23,  +0.525)   (+0.79,   ?, +1.23)
//!     FR  (-0.79,   -1.23,  +0.525)   (-0.79,   ?, +1.23)
//! ```
//!
//! Rear radius ~0.755 m, front radius ~0.525 m (read off the wheel
//! centres' height-above-ground in the USDA).

use datapod::{Point, Quaternion, Size};

use crate::vehicle::{
    ChassisSpec, MeshSource, PowerKind, PowerSource, VehicleBuilder, VehicleSpec, WheelSpec,
};

const MAX_STEER_RAD: f64 = 0.6109; // 35°

pub fn tractor_articulated() -> VehicleSpec {
    // Chassis silhouette — slightly larger than the original
    // `tractor` preset to match the USDA's wider, longer body.
    let chassis_x = 1.70_f64;
    let chassis_y = 1.50_f64;
    let chassis_z = 3.10_f64;

    let chassis = ChassisSpec {
        size: Size::new(chassis_x, chassis_y, chassis_z),
        // Sum of the rigid-body masses authored in the USDA
        // (chassis 2000 + 4 wheels 200/200/110/110 + 2 knuckles 5/5)
        // ≈ 2630 kg. Round to 2700 to give the chassis a bit of
        // extra inertia for the parts the raycast vehicle doesn't
        // model.
        mass: 2700.0,
        com_offset: Point::new(0.0, -0.45, 0.0),
        linear_damping: 0.2,
        angular_damping: 2.2,
        ccd: false,
        color: [0.0, 1.0, 0.392], // John Deere green — overridden by USD materials
        inertia_size: None,
        // The USD scene supplies all the visible body geometry. The
        // procedural chassis cuboid would float inside the imported
        // mesh and look wrong — suppress it.
        render_chassis: false,
        mesh: MeshSource::Box,
        // `bevy_openusd` exposes a `Handle<Scene>` indirectly via
        // `UsdAsset.scene`. Load `Handle<UsdAsset>` here (the loader's
        // primary type) and let `instantiate_pending_usd_scenes`
        // copy the inner scene handle onto a `SceneRoot` once load
        // completes.
        usd_asset: Some("machines/tractor_articulated.usda"),
        // USDA is authored with origin at the GROUND PLANE (wheel
        // bottoms at Y=0). Rapier's chassis frame has origin at the
        // chassis collider's centre. Translate the SceneRoot down by
        // (chassis_y/2 + clearance) so the asset's Y=0 lines up with
        // the rapier wheels' rest contact below the chassis.
        usd_scene_offset: Point::new(0.0, -(1.50 * 0.5 + 0.20), 0.0),
        usd_scene_rotation: Quaternion::identity(),
    };

    // Suspension. Tuned identically to the regular `tractor` preset
    // (which is proven to settle and drive correctly under load).
    // Earlier values (`stiffness = 80`, `rest = 0.05`) under-extended
    // the spring at this mass — the chassis bottomed out, dragging
    // its collider on the ground and starving the wheels of contact
    // force, which in turn killed traction (the visible "wheels spin
    // slow / vehicle doesn't move" symptom).
    let rest = 0.22;
    let stiffness = 90.0;
    let damping = 7.0;
    let friction = 24.0;
    let max_force = 30_000.0;

    // Wheel radii read from the USDA's wheel-centre heights
    // (the wheels sit on the ground in the source mesh, so
    // centre.z = radius after the 0.25× rescale).
    let front_radius = 0.525;
    let front_width = 0.30;
    let rear_radius = 0.755;
    let rear_width = 0.50;

    // Chassis-local wheel attach points. Strategy: put wheel BOTTOM
    // at chassis_y/2 + clearance below the chassis centre so the
    // wheels carry the body and the chassis collider never grinds on
    // ground (matches `tractor.rs`'s 0.30 m clearance pattern).
    //   wheel_bottom_local = -chassis_y/2 - clearance
    //   chassis_connection.y = wheel_bottom_local + rest + radius
    let clearance = 0.20_f64;
    let target_bottom = -chassis_y * 0.5 - clearance;
    let rear_conn_y = target_bottom + rest + rear_radius;
    let front_conn_y = target_bottom + rest + front_radius;

    let rear_wheel_x = 0.8475;
    let rear_z = -1.14;
    let front_wheel_x = 0.79;
    let front_z = 1.23;

    let front = |x: f64, prim: &'static str| WheelSpec {
        chassis_connection: Point::new(x, front_conn_y, front_z),
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
        max_brake: 700.0,
        max_steer_rad: MAX_STEER_RAD,
        steering_pivot_offset: Point::new(0.0, 0.0, 0.0),
        usd_prim_path: Some(prim),
        usd_steer_prim_path: None,
    };
    let rear = |x: f64, prim: &'static str| WheelSpec {
        chassis_connection: Point::new(x, rear_conn_y, rear_z),
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
        max_brake: 1800.0,
        max_steer_rad: 0.0,
        steering_pivot_offset: Point::new(0.0, 0.0, 0.0),
        usd_prim_path: Some(prim),
        usd_steer_prim_path: None,
    };

    // Wheel index → USD prim path (the asset's authored wheel-frame
    // Xform). After SceneRoot instantiates these subtrees,
    // `tag_usd_wheels` detaches each from the chassis hierarchy and
    // tags it with `VehicleWheel` so the standard sync system writes
    // the raycast pose into its world Transform.
    // Spec name is `"tractor"` (not `"tractor_articulated"`) so the
    // zenoh topic prefix `{name}_{id}` stays as `tractor_0`,
    // matching scripts authored against the old preset.
    VehicleBuilder::new("tractor", chassis)
        .max_speed(6.0)
        .wheel(front( front_wheel_x, "/robot/steer_front_left/wheel"))
        .wheel(front(-front_wheel_x, "/robot/steer_front_right/wheel"))
        .wheel(rear ( rear_wheel_x,  "/robot/wheel_back_left"))
        .wheel(rear (-rear_wheel_x,  "/robot/wheel_back_right"))
        // No `.part(...)` calls — the USD scene supplies all the
        // visible body geometry. Power source kept similar to the
        // procedural tractor.
        .power_source(
            PowerSource::new(PowerKind::Fuel, "Fuel", 300.0)
                .with_travel_drain(1.2)
                .with_work_drain(2.5),
        )
        .build()
}
