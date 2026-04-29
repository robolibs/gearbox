//! Chassis description — the rigid body at the core of a vehicle.

use datapod::{Point, Quaternion, Size};

use super::mesh::MeshSource;

/// Declarative description of a vehicle chassis.
#[derive(Debug, Clone)]
pub struct ChassisSpec {
    /// Full-size bounding box dimensions `(width, height, length)` in metres.
    pub size: Size,
    /// Total mass in kilograms.
    pub mass: f64,
    /// Local center-of-mass offset. A negative Y lowers the COM and reduces
    /// rollover; this matters a lot for a tall tractor on raycast wheels.
    pub com_offset: Point,
    pub linear_damping: f64,
    pub angular_damping: f64,
    /// Enable continuous collision detection on the chassis — prevents
    /// tunneling through walls at high speed.
    pub ccd: bool,
    /// Visual base colour (sRGB, 0..1). Used only by the viz layer; the
    /// library core ignores it entirely.
    pub color: [f32; 3],
    /// Override the box used for principal-inertia calculation. When
    /// `None`, uses `size` — which is the physics collider's extent.
    /// For gantry-style machines where the collider is a small central
    /// pod but the actual mass distribution spans metres, set this to
    /// the full outer bounding box so roll/pitch inertias aren't
    /// ridiculously small (which makes the body jump on every
    /// horizontal wheel force).
    pub inertia_size: Option<Size>,
    /// Whether the viz layer should render a mesh for the chassis box
    /// itself. For gantry machines (Robotti) the chassis is just a
    /// tiny hidden physics stub and the visible silhouette comes
    /// entirely from parts — turning this off suppresses the
    /// otherwise-visible "floating" chassis cuboid at the origin.
    pub render_chassis: bool,
    /// How the chassis itself should be rendered. `MeshSource::Box`
    /// (the default) renders a cuboid sized by `size`. Switch to a
    /// different variant to render a cylinder (or later, an external
    /// asset) without touching viz code.
    pub mesh: MeshSource,
    /// Optional path (relative to the binary's `assets/` directory)
    /// to a USD scene that supplies the visible body geometry. When
    /// set, the viz layer spawns the scene as a child of the chassis
    /// entity instead of (or in addition to) the procedural box +
    /// parts. Set `render_chassis = false` and supply no `parts(...)`
    /// to avoid double-bodywork. Loaded by `bevy_openusd`.
    pub usd_asset: Option<&'static str>,
    /// Local-frame translation applied to the spawned `SceneRoot`
    /// child carrying the USD scene. USDA assets are commonly authored
    /// with origin at the *ground plane* (wheel bottoms at Y=0), but
    /// rapier's chassis frame has origin at the chassis-collider
    /// centre. Set this to `(0, -(chassis_y/2 + clearance), 0)` so
    /// the asset's authored ground aligns with the rapier wheels'
    /// rest contact, otherwise the visible body floats above the
    /// physical chassis. Default `(0, 0, 0)` — fine for assets
    /// authored with origin at chassis centre.
    pub usd_scene_offset: Point,
    /// Rotation applied to the spawned `SceneRoot` child, on top of
    /// `bevy_openusd`'s internal Z-up→Y-up flip.
    ///
    /// Different USD authors pick different forward axes — e.g. our
    /// `tractor_articulated.usda` puts forward along `+Y` (= bevy
    /// `-Z` after the up-axis flip, matching gearbox's `+Z = forward`
    /// convention with no extra rotation), while typical URDF→USD
    /// pipelines (husky, robotti) use `+X = forward`. For the latter,
    /// set this to `Quaternion::from_axis_angle(Y, -π/2)` so the
    /// asset's authored forward lands on gearbox `+Z`.
    ///
    /// Default identity — fine for assets that already match
    /// gearbox conventions after the up-axis flip.
    pub usd_scene_rotation: Quaternion,
}

impl Default for ChassisSpec {
    fn default() -> Self {
        Self {
            size: Size::new(1.8, 0.8, 4.0),
            mass: 1400.0,
            com_offset: Point::new(0.0, -0.2, 0.0),
            linear_damping: 0.1,
            angular_damping: 0.7,
            ccd: true,
            color: [0.25, 0.55, 0.22],
            inertia_size: None,
            render_chassis: true,
            mesh: MeshSource::Box,
            usd_asset: None,
            usd_scene_offset: Point::new(0.0, 0.0, 0.0),
            usd_scene_rotation: Quaternion::identity(),
        }
    }
}
