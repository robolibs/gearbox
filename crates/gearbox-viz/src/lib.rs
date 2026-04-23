//! Bevy rendering layer for `gearbox_physics::Sim`.
//!
//! Wraps the headless sim in a Bevy resource, spawns PBR meshes that
//! mirror each [`gearbox_core::VehicleSpec`], keeps their transforms
//! in sync with the rapier world each frame, and owns the chase
//! camera + ground-grid machinery. Takes keyboard input for dev
//! teleop; external control (gamepad, joystick, scripted agents) is
//! expected to ride the robot-API layer (zenoh) rather than touch
//! this crate directly.
//!
//! The editor (gearbox-editor) layers on top of this crate — viz has
//! no awareness of selection, gizmos, panels, etc.

pub mod camera;
pub mod clouds;
pub mod grid;
pub mod input;
pub mod spawn;
pub mod step;
pub mod sync;
pub mod window_settings;

use bevy::prelude::*;

use gearbox_physics::Sim as CoreSim;
use gearbox_core::VehicleId;

/// Bevy resource wrapping a headless `gearbox_physics::Sim`. Renamed
/// from `Sim` so it doesn't shadow the library type.
#[derive(Resource)]
pub struct GearboxSim(pub CoreSim);

impl Default for GearboxSim {
    fn default() -> Self {
        Self(CoreSim::new())
    }
}

/// Tag on the vehicle whose WASD input drives it.
#[derive(Component, Default)]
pub struct PlayerControlled;

/// Which vehicle the chase camera should translate with. `None` means
/// no follow — the camera sits where the user left it. When set, a
/// per-frame system translates the camera focus (and world transform)
/// by the delta in the vehicle's XYZ position. No re-aim, no yaw
/// correction — if the user is pointing away from the machine, the
/// follow still works, they just won't see it.
#[derive(Resource, Default, Debug)]
pub struct FollowTarget {
    pub vehicle: Option<VehicleId>,
    /// Last observed world-position for the followed vehicle. `None`
    /// on a fresh follow (no previous frame to diff against) so the
    /// first tick contributes no delta.
    last_pos: Option<Vec3>,
}

impl FollowTarget {
    /// Set the follow target. Clears `last_pos` so the first tick
    /// after switching vehicles doesn't produce a big jump delta.
    pub fn set(&mut self, id: Option<VehicleId>) {
        self.vehicle = id;
        self.last_pos = None;
    }

    /// Call `set(None)` if `id` is already the target, otherwise
    /// `set(Some(id))`. Used by the outliner radio toggle.
    pub fn toggle(&mut self, id: VehicleId) {
        if self.vehicle == Some(id) {
            self.set(None);
        } else {
            self.set(Some(id));
        }
    }
}

/// Tag applied to the Bevy entity representing a vehicle's chassis.
#[derive(Component, Copy, Clone)]
pub struct VehicleBody {
    pub id: VehicleId,
}

/// Tag applied to the Bevy entity representing a single wheel.
#[derive(Component, Copy, Clone)]
pub struct VehicleWheel {
    pub id: VehicleId,
    pub index: usize,
}

/// Marks an entity whose material should re-tint when the user
/// changes a vehicle's chassis colour in the Properties panel.
/// Attached at spawn to the chassis mesh AND to every part whose
/// declared colour matches the chassis colour (i.e. cab, beams,
/// crossbars — the "bodywork"). Black roofs, dark hitches, contrast
/// stripes etc. carry their own colour and are NOT tagged.
#[derive(Component, Copy, Clone)]
pub struct ChassisTinted {
    pub id: VehicleId,
}

/// Insert on a Bevy `App` to wire gearbox → Bevy.
pub struct GearboxVizPlugin;

impl Plugin for GearboxVizPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GearboxSim>()
            .init_resource::<grid::GroundGrid>()
            .init_resource::<step::SimClock>()
            .init_resource::<FollowTarget>()
            .add_systems(
                Update,
                (
                    input::wasd_input_system,
                    step::step_sim_system,
                    sync::sync_vehicle_transforms_system,
                    follow_target_system,
                    camera::chase_camera_control,
                    camera::chase_camera_zoom,
                    camera::chase_camera_fly,
                )
                    .chain(),
            )
            .add_systems(Update, (grid::build_grid_meshes, grid::update_grid_alpha));
    }
}

/// Translates the chase camera with the followed vehicle — just
/// position, no yaw or look-at. If no follow target, or the target
/// has vanished, it no-ops. Runs after `sync_vehicle_transforms_system`
/// so poses are current for this frame.
pub fn follow_target_system(
    sim: Res<GearboxSim>,
    mut target: ResMut<FollowTarget>,
    mut cameras: Query<(&mut camera::ChaseCamera, &mut Transform)>,
) {
    let Some(id) = target.vehicle else {
        target.last_pos = None;
        return;
    };
    if sim.0.vehicle(id).is_none() {
        target.set(None);
        return;
    }
    let pose = sim.0.vehicle_pose(id);
    let current = Vec3::new(
        pose.point.x as f32,
        pose.point.y as f32,
        pose.point.z as f32,
    );
    if let Some(last) = target.last_pos {
        let delta = current - last;
        if delta.length_squared() > 0.0 {
            for (mut cam, mut tr) in &mut cameras {
                // Skip while a cinematic fly is still running — its
                // own system owns focus/yaw/distance that frame.
                if cam.fly_target.is_some() {
                    continue;
                }
                cam.focus += delta;
                tr.translation += delta;
            }
        }
    }
    target.last_pos = Some(current);
}

pub use camera::ChaseCamera;
pub use grid::GroundGrid;
pub use spawn::{spawn_height_for, spawn_vehicle_ghost, spawn_vehicle_visuals, GhostTag};
pub use step::{SimClock, SimSpeed};
