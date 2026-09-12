//! What the mara panes ask the world to do when it takes more than a
//! resource write: anything that needs queries (entities under a root, the
//! camera, prim lookups). The host pushes `HostCommand`s, the Bevy side
//! applies them at the start of the next update.

use std::path::PathBuf;

use bevy::asset::AssetId;
use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use mara::ui::modules::bevy::ChaseCamera;
use usd_bevy::UsdPrimRef;
use usd_bevy::instance::UsdInstanceOverrides;

use crate::controller::ControllerInventory;
use crate::load::LoadedAsset;
use crate::viewer::state::{
    ActiveStage, CameraBookmark, CameraBookmarks, ChaseCameraFly, FlyTarget, FlyTo, FollowTarget,
    LoadRequest, LoaderTuning, ReloadRequest, SelectedPrim,
};
use crate::viewer::systems::{
    PendingDespawn, Selection, find_loaded_ancestor, fit_params_for_entity, start_fly_to,
};

#[derive(Debug, Clone)]
pub enum HostCommand {
    /// Select a loaded asset root (and make it the active stage).
    SelectRoot(Option<Entity>),
    /// Select a prim; its owning root becomes the selection too.
    SelectPrim(Option<Entity>),
    FlyToPrim(Entity),
    FitPrim(Entity),
    FlyToMachine(Entity),
    ToggleFollow(Entity),
    SetVisibility(Entity, bool),
    Despawn(Entity),
    Reload,
    Load(PathBuf),
    /// Unload every runtime USD and pause.
    Clear,
    SetPhysics(bool),
    SetVariant {
        root: Entity,
        prim: String,
        set: String,
        selection: String,
    },
    SetMaterialColor(AssetId<StandardMaterial>, [f32; 3]),
    /// Move a paused asset's root: translation and yaw in world space.
    PoseRoot {
        root: Entity,
        translation: Vec3,
        yaw: f32,
    },
    SaveBookmark,
    RecallBookmark(usize),
    ClearBookmarks,
}

#[derive(Resource, Default)]
pub struct HostCommands(pub Vec<HostCommand>);

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct CommandState<'w> {
    selection: ResMut<'w, Selection>,
    active: ResMut<'w, ActiveStage>,
    selected: ResMut<'w, SelectedPrim>,
    fly: ResMut<'w, FlyTo>,
    machine_fly: ResMut<'w, ChaseCameraFly>,
    follow: ResMut<'w, FollowTarget>,
    despawn: ResMut<'w, PendingDespawn>,
    reload: ResMut<'w, ReloadRequest>,
    load: ResMut<'w, LoadRequest>,
    physics: ResMut<'w, gearbox_api::PhysicsActive>,
    bookmarks: ResMut<'w, CameraBookmarks>,
    tuning: ResMut<'w, LoaderTuning>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    inventory: Res<'w, ControllerInventory>,
    reset: MessageWriter<'w, gearbox_api::SimResetRequest>,
}

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct CommandQueries<'w, 's> {
    cameras: Query<'w, 's, (&'static mut ChaseCamera, &'static mut Transform)>,
    transforms: Query<'w, 's, &'static mut Transform, Without<ChaseCamera>>,
    globals: Query<'w, 's, &'static GlobalTransform>,
    aabbs: Query<'w, 's, &'static bevy::camera::primitives::Aabb>,
    children: Query<'w, 's, &'static Children>,
    parents: Query<'w, 's, &'static ChildOf>,
    loaded: Query<'w, 's, Entity, With<LoadedAsset>>,
    prims: Query<'w, 's, (Entity, &'static UsdPrimRef)>,
    visibility: Query<'w, 's, &'static mut Visibility>,
    overrides: Query<'w, 's, &'static mut UsdInstanceOverrides>,
}

pub(crate) fn apply_host_commands(
    mut queue: ResMut<HostCommands>,
    mut s: CommandState,
    mut q: CommandQueries,
) {
    if queue.0.is_empty() {
        return;
    }
    for command in std::mem::take(&mut queue.0) {
        match command {
            HostCommand::SelectRoot(root) => {
                s.selection.0 = root;
                if root.is_some() {
                    s.active.0 = root;
                }
            }
            HostCommand::SelectPrim(prim) => {
                s.selected.0 = prim;
                if let Some(prim) = prim
                    && let Some(root) = find_loaded_ancestor(prim, &q.parents, &q.loaded)
                {
                    s.selection.0 = Some(root);
                    s.active.0 = Some(root);
                }
            }
            HostCommand::FlyToPrim(prim) => {
                s.selected.0 = Some(prim);
                if let (Ok(gt), Ok((cam, _))) = (q.globals.get(prim), q.cameras.single()) {
                    let distance = (cam.distance * 0.25).clamp(0.2, 40.0);
                    start_fly_to(&mut s.fly, cam, gt.translation(), distance);
                }
            }
            HostCommand::FitPrim(prim) => {
                s.selected.0 = Some(prim);
                if let Ok((cam, _)) = q.cameras.single() {
                    let (focus, distance) = fit_params_for_entity(
                        prim,
                        &q.globals,
                        &q.aabbs,
                        &q.children,
                        cam.distance,
                    );
                    start_fly_to(&mut s.fly, cam, focus, distance);
                }
            }
            HostCommand::FlyToMachine(root) => {
                if let Ok((cam, _)) = q.cameras.single() {
                    let body = crate::viewer::systems::machine_body_entity(
                        root,
                        &s.inventory,
                        &q.prims,
                        &q.parents,
                    );
                    s.machine_fly.target = Some(FlyTarget::new(root, body, cam));
                }
            }
            HostCommand::ToggleFollow(root) => {
                s.follow.toggle(root);
                debug!("gearbox-viewer: follow toggled -> {:?}", s.follow.entity);
            }
            HostCommand::SetVisibility(entity, visible) => {
                if let Ok(mut v) = q.visibility.get_mut(entity) {
                    *v = if visible {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    };
                }
            }
            HostCommand::Despawn(root) => s.despawn.0.push(root),
            HostCommand::Reload => s.reload.requested = true,
            HostCommand::Load(path) => s.load.path = Some(path),
            HostCommand::Clear => {
                s.reset.write(gearbox_api::SimResetRequest {
                    pause_clock: true,
                    scope: gearbox_api::clear_scope::ALL,
                });
                s.physics.0 = false;
                s.selection.0 = None;
                s.selected.0 = None;
                s.active.0 = None;
            }
            HostCommand::SetPhysics(on) => s.physics.0 = on,
            HostCommand::SetVariant {
                root,
                prim,
                set,
                selection,
            } => {
                s.tuning
                    .variants
                    .insert((prim.clone(), set.clone()), selection.clone());
                if let Ok(mut ov) = q.overrides.get_mut(root) {
                    ov.variants.retain(|(p, st, _)| p != &prim || st != &set);
                    ov.variants.push((prim, set, selection));
                }
            }
            HostCommand::SetMaterialColor(id, rgb) => {
                if let Some(mut mat) = s.materials.get_mut(id) {
                    let alpha = mat.base_color.alpha();
                    mat.base_color = Color::linear_rgba(rgb[0], rgb[1], rgb[2], alpha);
                }
            }
            HostCommand::PoseRoot {
                root,
                translation,
                yaw,
            } => {
                if let Ok(mut tr) = q.transforms.get_mut(root) {
                    tr.translation = translation;
                    tr.rotation = Quat::from_rotation_y(yaw);
                }
            }
            HostCommand::SaveBookmark => {
                if let Ok((cam, _)) = q.cameras.single() {
                    s.bookmarks.next_seq += 1;
                    let name = format!("View {}", s.bookmarks.next_seq);
                    s.bookmarks.items.push(CameraBookmark {
                        name,
                        focus: cam.focus,
                        distance: cam.distance,
                        yaw: cam.yaw,
                        elevation: cam.elevation,
                    });
                }
            }
            HostCommand::RecallBookmark(index) => {
                if let (Some(b), Ok((cam, _))) =
                    (s.bookmarks.items.get(index), q.cameras.single())
                {
                    s.fly.start_focus = cam.focus;
                    s.fly.start_distance = cam.distance;
                    s.fly.target_focus = b.focus;
                    s.fly.target_distance = b.distance;
                    s.fly.start_yaw = Some(cam.yaw);
                    s.fly.target_yaw = Some(b.yaw);
                    s.fly.start_elevation = Some(cam.elevation);
                    s.fly.target_elevation = Some(b.elevation);
                    s.fly.duration = 0.6;
                    s.fly.remaining = 0.6;
                }
            }
            HostCommand::ClearBookmarks => s.bookmarks.items.clear(),
        }
    }
}
