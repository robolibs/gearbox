//! `probe-usd <path/to/file.usdc>` — dumps every prim's path + local
//! translate so we can wire up `usd_prim_path` mappings in presets.
//!
//! Runs headless (no DefaultPlugins, just the asset/scene plumbing
//! bevy_openusd needs).

use std::path::PathBuf;

use bevy::asset::{AssetPlugin, AssetServer, Assets, LoadState};
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::*;
use bevy::scene::{ScenePlugin, SceneRoot};
use bevy_openusd::{UsdAsset, UsdLoaderSettings, UsdPlugin, UsdPrimRef};

#[derive(Resource)]
struct UsdHandle(Handle<UsdAsset>);

#[derive(Resource, Default)]
struct State {
    spawned: bool,
    dumped: bool,
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe-usd <path/to/file.usd[ac]>");
    let pb = PathBuf::from(&path);
    let parent = pb
        .parent()
        .expect("file has a parent dir")
        .to_path_buf();
    let filename = pb.file_name().expect("file has a name").to_string_lossy().into_owned();
    println!("probing: {}", path);
    println!("asset_root: {}", parent.display());

    // Direct stage probe — read upAxis & metersPerUnit before bevy
    // even gets involved.
    if let Ok(stage) = openusd::Stage::open(&path) {
        let up = stage
            .field::<String>(openusd::sdf::Path::abs_root(), "upAxis")
            .ok()
            .flatten();
        let mpu = stage
            .field::<f64>(openusd::sdf::Path::abs_root(), "metersPerUnit")
            .ok()
            .flatten();
        println!("STAGE: upAxis={:?}, metersPerUnit={:?}", up, mpu);
    } else {
        println!("STAGE: failed to open via openusd::Stage");
    }

    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin {
            file_path: parent.to_string_lossy().into_owned(),
            ..default()
        })
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        // ScenePlugin's spawner panics if any component on a scene
        // entity isn't registered for reflection. Pre-register the
        // PBR material handle component so that headless probes
        // (no DefaultPlugins / PbrPlugin) don't crash.
        .register_type::<MeshMaterial3d<StandardMaterial>>()
        .add_plugins(ScenePlugin)
        .add_plugins(UsdPlugin)
        .insert_resource(State::default())
        .insert_resource(LoadedFile(filename, parent))
        .add_systems(Startup, kick_load)
        .add_systems(Update, (spawn_when_ready, dump_then_exit))
        .run();
}

#[derive(Resource)]
struct LoadedFile(String, PathBuf);

fn kick_load(mut commands: Commands, asset_server: Res<AssetServer>, file: Res<LoadedFile>) {
    let parent = file.1.clone();
    let h: Handle<UsdAsset> = asset_server.load_with_settings(
        file.0.clone(),
        move |s: &mut UsdLoaderSettings| {
            s.search_paths = vec![parent.clone()];
        },
    );
    commands.insert_resource(UsdHandle(h));
}

fn spawn_when_ready(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    assets: Res<Assets<UsdAsset>>,
    handle: Option<Res<UsdHandle>>,
    mut state: ResMut<State>,
) {
    if state.spawned {
        return;
    }
    let Some(handle) = handle else { return };
    match asset_server.get_load_state(&handle.0) {
        Some(LoadState::Loaded) => {
            let Some(asset) = assets.get(&handle.0) else { return };
            println!(
                "loaded UsdAsset: default_prim={:?}, layer_count={}, joints={}, rigid_bodies={}",
                asset.default_prim,
                asset.layer_count,
                asset.joints.len(),
                asset.rigid_body_prims.len(),
            );
            for j in &asset.joints {
                println!(
                    "  joint: {} kind={:?} body0={:?} body1={:?} axis={:?} local_pos1={:?}",
                    j.path, j.kind, j.body0, j.body1, j.axis, j.local_pos1,
                );
            }
            for rb in &asset.rigid_body_prims {
                println!("  rigid_body: {}", rb);
            }
            commands.spawn((SceneRoot(asset.scene.clone()), Transform::default()));
            state.spawned = true;
        }
        Some(LoadState::Failed(err)) => {
            eprintln!("UsdAsset load FAILED: {err}");
            std::process::exit(1);
        }
        _ => {}
    }
}

fn dump_then_exit(
    state: ResMut<State>,
    mut state_mut: Local<u32>,
    prims: Query<(&UsdPrimRef, &Transform)>,
) {
    if !state.spawned {
        return;
    }
    *state_mut += 1;
    // Wait a few ticks for the scene spawner to project everything.
    if *state_mut < 8 {
        return;
    }
    println!("\n=== {} prim entities projected ===", prims.iter().count());
    let mut rows: Vec<(String, Vec3, Quat, Vec3)> = prims
        .iter()
        .map(|(pr, tr)| (pr.path.clone(), tr.translation, tr.rotation, tr.scale))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, t, r, s) in rows {
        let non_identity = r.length_squared() > 0.0 && (r.w - 1.0).abs() > 1e-4;
        let non_unit_scale = (s - Vec3::ONE).abs().max_element() > 1e-4;
        if non_identity || non_unit_scale {
            println!(
                "  {:60} translate=({:+.3}, {:+.3}, {:+.3}) rot=(w={:.3} x={:.3} y={:.3} z={:.3}) scale=({:.3},{:.3},{:.3})",
                path, t.x, t.y, t.z, r.w, r.x, r.y, r.z, s.x, s.y, s.z
            );
        } else {
            println!("  {:60} translate=({:+.3}, {:+.3}, {:+.3})", path, t.x, t.y, t.z);
        }
    }
    std::process::exit(0);
}
