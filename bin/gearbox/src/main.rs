//! gearbox — USD simulator with the full bevy_openusd viewer panel
//! set folded in on top of the planet world / multi-USD loader /
//! play button.
//!
//! The simulator-specific surface (planet sphere, ChaseCamera +
//! GroundGrid wiring, multi-USD `LoadQueue`, click-to-select) lives in
//! `world` + `load`. The viewer-side panel
//! set + selection-prim / fly-to / overlays / log capture / variants
//! / cameras / materials live in the `viewer` submodule, ported from
//! `bevy_openusd::*`.

#![allow(
    dead_code,
    clippy::collapsible_if,
    clippy::field_reassign_with_default,
    clippy::needless_borrow,
    clippy::needless_borrows_for_generic_args,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_cast,
    clippy::unnecessary_map_or,
    clippy::useless_conversion
)]

mod controller;
mod load;
mod viewer;
mod world;

use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use bevy_mara::MaraPlugin;

fn main() {
    let cli_paths: Vec<std::path::PathBuf> = std::env::args()
        .skip(1)
        .map(|s| {
            let p = std::path::PathBuf::from(&s);
            if p.is_absolute() {
                p
            } else {
                std::env::current_dir().unwrap_or_default().join(p)
            }
        })
        .collect();

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "gearbox — USD simulator".to_string(),
                        resolution: (1400, 900).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: "/".to_string(),
                    unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow,
                    ..default()
                })
                .set(LogPlugin {
                    custom_layer: viewer::log_panel::loader_log_custom_layer,
                    ..default()
                }),
        )
        // Wireframe support for the Overlays panel toggle. Without
        // this, `WireframeConfig` doesn't exist as a resource and
        // the overlays sync system would panic on first frame.
        .add_plugins(bevy::pbr::wireframe::WireframePlugin::default())
        // ── UI stack: egui + Mara (glass theme + ribbons + widgets).
        .add_plugins(EguiPlugin::default())
        .add_plugins(MaraPlugin)
        // ── USD pipeline + rapier physics + animation playback.
        .add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::physics::RapierAdapterPlugin)
        .add_plugins(usd_bevy::anim::AnimPlugin)
        .insert_resource(usd_bevy::physics::PhysicsActive(false))
        // ── Tool API: one host agent per process, one agent per machine.
        // Identity, allowlist, and relay come from GEARBOX_* env vars.
        .add_plugins(gearbox_api::GearboxBusPlugin {
            config: gearbox_api::HostConfig::from_env(env!("CARGO_PKG_VERSION")),
        })
        .insert_resource(gearbox_api::UsdAssetRoot(load::default_asset_root()))
        .add_plugins(gearbox_api::UsdLoaderPlugin)
        .add_plugins(gearbox_api::UsdMarkerPlugin)
        // ── Simulator surface: persistent planet world + the multi-
        // USD `LoadQueue`-driven loader.
        .add_plugins(world::WorldPlugin)
        .add_plugins(controller::ControllerDiscoveryPlugin)
        .add_plugins(load::LoadPlugin { cli_paths })
        // ── Viewer surface: full ribbon + panel set, overlays, prim
        // tree, prim-level selection, fly-to camera, log capture,
        // variants, cameras, materials.
        .add_plugins(viewer::ui::ViewerUiPlugin)
        .add_plugins(viewer::keyboard::ViewerKeyboardPlugin)
        .add_plugins(viewer::overlays::OverlaysPlugin)
        .add_plugins(viewer::physics_overlay::PhysicsOverlayPlugin)
        .run();
}
