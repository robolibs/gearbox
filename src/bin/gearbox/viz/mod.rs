//! Bevy-based visualization for `gearbox::Sim`.
//!
//! Lives in the binary, not the library, so the library stays Bevy-free.

pub mod camera;
pub mod clouds;
pub mod grid;
pub mod input;
pub mod spawn;
pub mod step;
pub mod sync;

use bevy::prelude::*;

use gearbox::Sim as CoreSim;

/// Bevy resource wrapping a headless `gearbox::Sim`. Renamed from `Sim`
/// so it doesn't shadow `gearbox::Sim`.
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

/// Tag applied to the Bevy entity representing a vehicle's chassis.
#[derive(Component, Copy, Clone)]
pub struct VehicleBody {
    pub id: gearbox::VehicleId,
}

/// Tag applied to the Bevy entity representing a single wheel.
#[derive(Component, Copy, Clone)]
pub struct VehicleWheel {
    pub id: gearbox::VehicleId,
    pub index: usize,
}

/// Insert on a Bevy `App` to wire gearbox → Bevy.
pub struct GearboxVizPlugin;

impl Plugin for GearboxVizPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GearboxSim>()
            .init_resource::<grid::GroundGrid>()
            .add_systems(
                Update,
                (
                    input::sync_player_to_selection_system,
                    input::wasd_input_system,
                    step::step_sim_system,
                    sync::sync_vehicle_transforms_system,
                    camera::chase_camera_control,
                    camera::chase_camera_zoom,
                )
                    .chain(),
            )
            .add_systems(Update, (grid::build_grid_meshes, grid::update_grid_alpha));
    }
}

pub use camera::ChaseCamera;
pub use grid::GroundGrid;
pub use spawn::{spawn_height_for, spawn_vehicle_ghost, spawn_vehicle_visuals, GhostTag};
