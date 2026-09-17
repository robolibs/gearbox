//! The embedded Bevy app: every simulator plugin, wired into the app the
//! mara viewport owns. There is no window here; mara's renderer builds the
//! `DefaultPlugins` without winit and hands the camera a render target.

use std::path::PathBuf;

use bevy::app::PluginGroupBuilder;
use bevy::asset::{AssetPlugin, UnapprovedPathMode};
use bevy::prelude::*;
use mara::ui::modules::bevy::BevyViewportInput;

use crate::{
    attach, controller, environment, load, physics, physics_debug, services, terrain, viewer,
    world,
};

/// USD files are addressed by absolute path, so the asset root is `/`.
pub fn configure_plugins(group: PluginGroupBuilder) -> PluginGroupBuilder {
    group.set(AssetPlugin {
        file_path: "/".to_string(),
        unapproved_path_mode: UnapprovedPathMode::Allow,
        ..default()
    })
}

pub fn configure(app: &mut App, cli_paths: Vec<PathBuf>, wireframe_supported: bool) {
    app.insert_resource(viewer::overlays::WireframeAvailable(wireframe_supported));
    if wireframe_supported {
        app.add_plugins(bevy::pbr::wireframe::WireframePlugin::default());
    } else {
        app.init_resource::<bevy::pbr::wireframe::WireframeConfig>();
        warn!("wireframe unavailable: shared GPU device lacks line mode or 16-byte immediates");
    }
    app.init_resource::<BevyViewportInput>()
        // Frame time and fps land in the log every ten seconds.
        .add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::diagnostic::LogDiagnosticsPlugin {
                wait_duration: std::time::Duration::from_secs(10),
                ..default()
            },
        ))
        // USD pipeline (live stage projection) + gearbox's rapier world.
        .add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::asset::UsdAssetPlugin)
        // Accumulates open / override / validation / projection time per
        // load; the loader logs it when a root becomes ready.
        .init_resource::<usd_bevy::asset::UsdSceneTimings>()
        .insert_resource(projection_budget_from_env())
        .add_plugins(physics::PhysicsPlugin)
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
        .add_plugins(environment::EnvironmentPlugin)
        .add_plugins(gearbox_fields::FieldsPlugin::default())
        .add_plugins(terrain::TerrainPlugin)
        .configure_sets(
            Update,
            gearbox_fields::CoverUpdates.after(terrain::TerrainUpdates),
        )
        .add_systems(
            Update,
            (
                terrain::publish_cover_terrain.in_set(terrain::TerrainUpdates),
                environment::sync_cover_wind,
            ),
        )
        .add_systems(Startup, (log_render_adapter, use_cpu_light_clustering))
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
        .add_plugins(viewer::recorder::RecorderPlugin)
        .add_plugins(viewer::overlays::OverlaysPlugin)
        .add_plugins(viewer::physics_overlay::PhysicsOverlayPlugin);
}

/// How much of a frame the loader may spend projecting prims:
/// `GEARBOX_PROJECTION_BUDGET_MS`, unbounded by default. usd_bevy's 10 ms
/// keeps an editor smooth but stretches a 4000-prim machine over a minute,
/// and the stage parse blocks the frame anyway, so a simulator finishes the
/// projection in that same frame.
/// Which GPU and backend the window renders with; the wrong one is the
/// first thing to rule out when frames are slow.
/// The scene is lit by the sun, which needs no clusters. Bevy's GPU
/// clustering rebuilt its buffers every frame (~30 ms at a far plane of
/// 80 km); CPU clustering still serves any point or spot lights a USD brings.
fn use_cpu_light_clustering(settings: Option<ResMut<bevy::light::cluster::GlobalClusterSettings>>) {
    if let Some(mut settings) = settings {
        settings.gpu_clustering = None;
        info!("light clustering: CPU");
    }
}

fn log_render_adapter(adapter: Option<Res<bevy::render::renderer::RenderAdapterInfo>>) {
    match adapter {
        Some(info) => info!(
            "render adapter: {} ({:?}, {:?}, driver {})",
            info.name, info.backend, info.device_type, info.driver_info
        ),
        None => warn!("render adapter: unknown"),
    }
}

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
