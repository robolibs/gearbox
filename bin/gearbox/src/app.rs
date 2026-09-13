//! The embedded Bevy app: every simulator plugin, wired into the app the
//! mara viewport owns. There is no window here; mara's renderer builds the
//! `DefaultPlugins` without winit and hands the camera a render target.

use std::path::PathBuf;

use bevy::app::PluginGroupBuilder;
use bevy::asset::{AssetPlugin, UnapprovedPathMode};
use bevy::prelude::*;
use mara::ui::modules::bevy::BevyViewportInput;

use crate::{attach, controller, load, physics, physics_debug, services, viewer, world};

/// USD files are addressed by absolute path, so the asset root is `/`.
pub fn configure_plugins(group: PluginGroupBuilder) -> PluginGroupBuilder {
    group.set(AssetPlugin {
        file_path: "/".to_string(),
        unapproved_path_mode: UnapprovedPathMode::Allow,
        ..default()
    })
}

pub fn configure(app: &mut App, cli_paths: Vec<PathBuf>) {
    app.init_resource::<BevyViewportInput>()
        // Wireframe support for the Overlays pane toggle; without it the
        // `WireframeConfig` resource would not exist.
        .add_plugins(bevy::pbr::wireframe::WireframePlugin::default())
        // USD pipeline (live stage projection) + gearbox's rapier world.
        .add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::asset::UsdAssetPlugin)
        // Accumulates open / override / validation / projection time per
        // load; the loader logs it when a root becomes ready.
        .init_resource::<usd_bevy::asset::UsdSceneTimings>()
        .insert_resource(projection_budget_from_env())
        .add_plugins(physics::RapierAdapterPlugin)
        .insert_resource(gearbox_api::PhysicsActive(false))
        // Tool API: one host agent per process, one agent per machine.
        .add_plugins(gearbox_api::GearboxBusPlugin {
            config: gearbox_api::HostConfig::from_env(env!("CARGO_PKG_VERSION")),
        })
        .insert_resource(gearbox_api::UsdAssetRoot(load::default_asset_root()))
        .add_plugins(gearbox_api::UsdLoaderPlugin)
        .add_plugins(gearbox_api::UsdMarkerPlugin)
        // Simulator surface: planet world, machines, links, services.
        .add_plugins(world::WorldPlugin)
        .add_plugins(controller::ControllerDiscoveryPlugin)
        .add_plugins(attach::AttachPlugin)
        .add_plugins(services::ServicesPlugin)
        .add_plugins(physics_debug::PhysicsDebugPlugin)
        .add_plugins(load::LoadPlugin { cli_paths })
        // Viewer state the mara panes read and drive.
        .add_plugins(viewer::systems::ViewerSystemsPlugin)
        .add_plugins(viewer::drive::DrivePlugin)
        .add_plugins(viewer::camera_requests::CameraRequestsPlugin)
        .add_plugins(viewer::tf_overlay::TfOverlayPlugin)
        .add_plugins(viewer::screenshot::ScreenshotPlugin)
        .add_plugins(viewer::overlays::OverlaysPlugin)
        .add_plugins(viewer::physics_overlay::PhysicsOverlayPlugin);
}

/// How much of a frame the loader may spend projecting prims:
/// `GEARBOX_PROJECTION_BUDGET_MS`, unbounded by default. usd_bevy's 10 ms
/// keeps an editor smooth but stretches a 4000-prim machine over a minute,
/// and the stage parse blocks the frame anyway, so a simulator finishes the
/// projection in that same frame.
fn projection_budget_from_env() -> usd_bevy::UsdProjectionBudget {
    const DEFAULT_MS: u64 = 0;
    let millis = std::env::var("GEARBOX_PROJECTION_BUDGET_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MS);
    let budget = if millis == 0 {
        std::time::Duration::MAX
    } else {
        std::time::Duration::from_millis(millis)
    };
    usd_bevy::UsdProjectionBudget(budget)
}
