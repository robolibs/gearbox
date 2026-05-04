//! `usd-world-demo <path1.usd> [path2.usd ...]` — load one or more
//! USD assets into a gearbox-shaped Bevy app.
//!
//! Layout: assets are arranged in a roughly-square grid on the XZ
//! plane around the origin, `MOUNT_SPACING` metres between cells.
//! The world layer (sky, sun, grid, axes, ground collider) comes
//! from `gearbox-world`; USD loading + physics come from `usd_bevy`.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::scene::SceneRoot;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_frost::prelude::*;
use bevy_frost::{style, FrostPlugin};
use gearbox_world::{WorldConfig, WorldPlugin};
use usd_bevy::anim::{AnimPlugin, UsdStageTime};
use usd_bevy::physics::{PhysicsActive, RapierAdapterPlugin};
use usd_bevy::{UsdAsset, UsdLoaderSettings, UsdPlugin};

// ── bevy_frost ribbon declaration ────────────────────────────────
const RIBBON_LEFT: &str = "demo_left";
const RIB_PLAY: &str = "demo_play";       // Toggle button — physics ▶/⏸
const RIB_TIMELINE: &str = "demo_timeline"; // Floating panel — anim transport
const RIB_ASSETS: &str = "demo_assets";   // Floating panel — loaded asset list

const RIBBONS: &[RibbonDef] = &[RibbonDef {
    id: RIBBON_LEFT,
    edge: RibbonEdge::Left,
    role: RibbonRole::Panel,
    mode: RibbonMode::TwoSided,
    draggable: false,
    accepts: &[],
}];

const RIBBON_ITEMS: &[RibbonItem] = &[
    RibbonItem {
        id: RIB_PLAY,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 0,
        glyph: RibbonGlyph::Text("▶"),
        tooltip: "Physics ▶/⏸",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_TIMELINE,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 1,
        glyph: RibbonGlyph::Text("T"),
        tooltip: "Timeline / Anim",
        child_ribbon: None,
    },
    RibbonItem {
        id: RIB_ASSETS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 2,
        glyph: RibbonGlyph::Text("A"),
        tooltip: "Loaded assets",
        child_ribbon: None,
    },
];

const PANEL_W: f32 = 320.0;

/// Distance between grid cells in metres. Tuned to keep typical
/// robot-scale assets (franka ~ 1.5 m, tractor ~ 4 m) clearly
/// separated without scattering them across the visible area.
const MOUNT_SPACING: f32 = 4.0;

/// Lay N points out on a roughly-square grid on the XZ plane,
/// centred on the origin. Returns one position per index `0..n`.
fn grid_positions(n: usize) -> Vec<Vec3> {
    if n == 0 {
        return Vec::new();
    }
    let cols = (n as f32).sqrt().ceil() as usize;
    let rows = (n + cols - 1) / cols;
    let half_w = (cols as f32 - 1.0) * 0.5;
    let half_h = (rows as f32 - 1.0) * 0.5;
    (0..n)
        .map(|i| {
            let r = i / cols;
            let c = i % cols;
            Vec3::new(
                (c as f32 - half_w) * MOUNT_SPACING,
                0.0,
                (r as f32 - half_h) * MOUNT_SPACING,
            )
        })
        .collect()
}

#[derive(Resource, Clone)]
struct PendingLoads(Vec<PendingAsset>);

#[derive(Clone)]
struct PendingAsset {
    abs_path: PathBuf,
    mount_point: Vec3,
    search_path: PathBuf,
}

#[derive(Resource, Default)]
struct LoadedAssets(Vec<LoadedAsset>);

struct LoadedAsset {
    handle: Handle<UsdAsset>,
    mount_point: Vec3,
    spawned: bool,
    label: String,
}

fn main() {
    let raw_paths: Vec<String> = std::env::args().skip(1).collect();
    if raw_paths.is_empty() {
        eprintln!("usage: usd-world-demo <path1.usd> [path2.usd ...]");
        std::process::exit(2);
    }

    let positions = grid_positions(raw_paths.len());
    let pending: Vec<PendingAsset> = raw_paths
        .iter()
        .zip(positions.into_iter())
        .map(|(raw, pos)| {
            let path = PathBuf::from(raw);
            let abs = if path.is_absolute() {
                path
            } else {
                std::env::current_dir().unwrap().join(path)
            };
            let parent = abs.parent().expect("asset has parent dir").to_path_buf();
            PendingAsset {
                abs_path: abs,
                mount_point: pos,
                search_path: parent,
            }
        })
        .collect();

    let title = format!(
        "usd-world-demo — {}",
        pending
            .iter()
            .map(|p| p.abs_path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" + ")
    );

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title,
                    resolution: (1400u32, 900u32).into(),
                    ..default()
                }),
                ..default()
            })
            // Asset root `/` so per-asset absolute paths work without
            // a common-ancestor calculation. Bevy 0.18 forbids paths
            // outside `file_path` by default — `Allow` is required
            // for arbitrary absolute load paths.
            .set(AssetPlugin {
                file_path: "/".to_string(),
                unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow,
                ..default()
            }),
    )
    // Pull the camera back enough to fit the whole grid; +4 m floor
    // so a single-asset run still looks normal.
    .add_plugins(WorldPlugin {
        config: WorldConfig {
            camera_distance: 4.0
                + (raw_paths.len() as f32).sqrt().ceil() * MOUNT_SPACING * 0.7,
            ..default()
        },
    })
    .add_plugins(EguiPlugin::default())
    .add_plugins(FrostPlugin)
    .add_plugins(UsdPlugin)
    .add_plugins(AnimPlugin)
    .add_plugins(RapierAdapterPlugin)
    .insert_resource(PhysicsActive(true))
    .insert_resource(PendingLoads(pending))
    .init_resource::<LoadedAssets>()
    .add_systems(Startup, request_usd_loads)
    .add_systems(Update, spawn_scenes_when_loaded)
    .add_systems(
        EguiPrimaryContextPass,
        (draw_ribbons, draw_timeline_panel, draw_assets_panel).chain(),
    );

    app.run();
}

fn request_usd_loads(
    asset_server: Res<AssetServer>,
    pending: Res<PendingLoads>,
    mut loaded: ResMut<LoadedAssets>,
) {
    for asset in &pending.0 {
        let search = vec![asset.search_path.clone()];
        let load_path = asset.abs_path.to_string_lossy().into_owned();
        let handle: Handle<UsdAsset> = asset_server.load_with_settings::<UsdAsset, _>(
            load_path.clone(),
            move |s: &mut UsdLoaderSettings| {
                s.search_paths = search.clone();
            },
        );
        let label = asset
            .abs_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| load_path.clone());
        info!("requested USD load: {label} at mount={:?}", asset.mount_point);
        loaded.0.push(LoadedAsset {
            handle,
            mount_point: asset.mount_point,
            spawned: false,
            label,
        });
    }
}

// ── bevy_frost ribbon + panels ──────────────────────────────────

fn draw_ribbons(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut open: ResMut<RibbonOpen>,
    mut placement: ResMut<RibbonPlacement>,
    mut drag: ResMut<RibbonDrag>,
    mut physics: ResMut<PhysicsActive>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let physics_on = physics.0;
    let clicks = draw_assembly(
        ctx,
        accent.0,
        RIBBONS,
        RIBBON_ITEMS,
        &mut open,
        &mut placement,
        &mut drag,
        // RIB_PLAY behaves as a toggle pill (no panel) — light up
        // when physics is running.
        |id| id == RIB_PLAY && physics_on,
    );
    for click in clicks {
        if click.item == RIB_PLAY {
            physics.0 = !physics.0;
        }
    }
}

fn is_panel_open(open: &RibbonOpen, item: &'static str) -> bool {
    open.is_open(RIBBON_LEFT, item)
}

fn draw_timeline_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    mut clock: ResMut<UsdStageTime>,
) {
    if !is_panel_open(&open, RIB_TIMELINE) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let mut keep = true;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_TIMELINE,
        "Timeline",
        egui::vec2(PANEL_W, 280.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("playback", "Playback", true, |ui| {
                sub_caption(
                    ui,
                    &format!(
                        "{:.1} fps · {:.2}s total",
                        clock.time_codes_per_second,
                        clock.duration_seconds()
                    ),
                );
                ui.add_space(style::space::BLOCK);

                let play_label = if clock.playing { "⏸  Pause" } else { "▶  Play" };
                if wide_button(ui, play_label, accent_col).clicked() {
                    clock.playing = !clock.playing;
                }
                if wide_button(ui, "⏮  Rewind", accent_col).clicked() {
                    clock.seconds = 0.0;
                }

                ui.add_space(style::space::BLOCK);
                let dur = clock.duration_seconds().max(1e-3);
                let _ = pretty_slider(
                    ui,
                    "Seconds",
                    &mut clock.seconds,
                    0.0..=dur,
                    3,
                    " s",
                    accent_col,
                );

                readout_row(ui, "timeCode", &format!("{:.3}", clock.current_time_code()));
                readout_row(
                    ui,
                    "range",
                    &format!("{:.2} … {:.2}", clock.start_time_code, clock.end_time_code),
                );
            });
        },
    );
}

fn draw_assets_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    loaded: Res<LoadedAssets>,
) {
    if !is_panel_open(&open, RIB_ASSETS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let mut keep = true;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_ASSETS,
        "Loaded Assets",
        egui::vec2(PANEL_W, 360.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("assets_list", "Mounted USDs", true, |ui| {
                sub_caption(ui, &format!("{} asset(s)", loaded.0.len()));
                ui.add_space(style::space::BLOCK);
                for entry in &loaded.0 {
                    let status = if entry.spawned { "●" } else { "○" };
                    readout_row(
                        ui,
                        &format!("{status} {}", entry.label),
                        &format!(
                            "({:.1}, {:.1}, {:.1})",
                            entry.mount_point.x, entry.mount_point.y, entry.mount_point.z
                        ),
                    );
                }
            });
        },
    );
}

fn spawn_scenes_when_loaded(
    mut commands: Commands,
    mut loaded: ResMut<LoadedAssets>,
    usd_assets: Res<Assets<UsdAsset>>,
) {
    for entry in loaded.0.iter_mut() {
        if entry.spawned {
            continue;
        }
        let Some(asset) = usd_assets.get(&entry.handle) else {
            continue;
        };
        commands.spawn((
            SceneRoot(asset.scene.clone()),
            Transform::from_translation(entry.mount_point),
        ));
        entry.spawned = true;
        info!(
            "spawned {} at {:?}: default_prim={:?}, layer_count={}",
            entry.label, entry.mount_point, asset.default_prim, asset.layer_count
        );
    }
}
