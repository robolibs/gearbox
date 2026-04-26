//! Bridge between gearbox-editor's selection / sim and the upstream
//! [`transform_gizmo_bevy`] crate (re-exported from
//! [`bevy_glacial`]).
//!
//! ## Surface
//!
//! Editor-side resources the UI panels read / write:
//! * [`GizmoModesEnabled`] — translate / rotate / scale toggles.
//! * [`GizmoScale`] — handle pixel size (maps to `GizmoVisuals.gizmo_size`).
//!
//! ## Wiring
//!
//! Add [`EditorGizmoBridgePlugin`]. It pulls in the upstream
//! [`TransformGizmoPlugin`], spawns one invisible "proxy" entity that
//! the gizmo manipulates, and runs the four glue systems:
//!
//!   1. `apply_gizmo_options` — `PreUpdate`, copies UI toggles + sim
//!      pause-state + egui-pointer-block into [`GizmoOptions`].
//!   2. `manage_proxy_target` — `PreUpdate`, attaches / detaches
//!      [`GizmoTarget`] on the proxy as the selection changes.
//!   3. `pull_proxy_from_sim` — `PreUpdate`, copies sim pose into the
//!      proxy's `Transform` whenever the gizmo isn't actively being
//!      dragged. Runs in `Last` so it's after the gizmo plugin.
//!   4. `push_proxy_to_sim` — `Last`, writes the proxy's `Transform`
//!      back to the sim while the gizmo is active.
//!
//! The gizmo crate handles all picking / drag math / rendering — this
//! file only translates between editor state and the gizmo's own
//! resources.

use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use bevy_egui::EguiContexts;

use bevy_glacial::{
    auto_scale_gizmo_to_target, EnumSet, GizmoAutoScale, GizmoMode, GizmoOptions, GizmoTarget,
    TransformGizmoPlugin,
};

use gearbox_physics::datapod::{Point, Pose, Quaternion};
use gearbox_viz::{GearboxSim, SimClock};

use super::selection::Selection;

/// Per-mode toggles surfaced by the Properties panel. Cleared modes
/// simply don't appear in the gizmo.
#[derive(Resource, Debug, Clone, Copy)]
pub struct GizmoModesEnabled {
    pub translate: bool,
    pub rotate: bool,
    pub scale: bool,
}

impl Default for GizmoModesEnabled {
    fn default() -> Self {
        Self {
            translate: true,
            rotate: true,
            scale: false,
        }
    }
}

/// Pixel-size multiplier for the gizmo. The gizmo stays at a roughly
/// constant *screen* size regardless of camera distance (standard
/// Blender / Maya behaviour); this multiplier lets the user dial it
/// up or down. `1.0` = the [`GIZMO_BASE_PX`] default.
#[derive(Resource, Debug, Clone, Copy)]
pub struct GizmoScale(pub f32);

impl Default for GizmoScale {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Pixel size that `GizmoScale(1.0)` resolves to. Picked so the
/// handles read clearly without dwarfing a typical vehicle.
pub const GIZMO_BASE_PX: f32 = 70.0;

/// Marker on the single proxy entity whose `Transform` mirrors the
/// selected vehicle's pose and gets a [`GizmoTarget`] attached when
/// the gizmo should be active.
#[derive(Component)]
struct GizmoProxy;

pub struct EditorGizmoBridgePlugin;

impl Plugin for EditorGizmoBridgePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TransformGizmoPlugin)
            .init_resource::<GizmoModesEnabled>()
            .init_resource::<GizmoScale>()
            .insert_resource(GizmoOptions {
                // Hotkeys would clash with the editor's UI controls.
                hotkeys: None,
                ..default()
            })
            .insert_resource(GizmoAutoScale {
                // GizmoScale slider is the user-facing knob; multiply
                // it through the auto-scale ceiling so the slider still
                // reads as "size".
                ..default()
            })
            .add_systems(Startup, spawn_proxy)
            .add_systems(
                PreUpdate,
                (
                    apply_gizmo_options,
                    manage_proxy_target,
                    sync_proxy_aabb,
                    pull_proxy_from_sim,
                )
                    .chain(),
            )
            .add_systems(Update, auto_scale_gizmo_to_target)
            .add_systems(Last, push_proxy_to_sim);
    }
}

fn spawn_proxy(mut commands: Commands) {
    commands.spawn((
        Name::new("GizmoProxy"),
        Transform::default(),
        GlobalTransform::default(),
        Visibility::Hidden,
        GizmoProxy,
    ));
}

fn apply_gizmo_options(
    modes: Res<GizmoModesEnabled>,
    scale: Res<GizmoScale>,
    clock: Res<SimClock>,
    mut contexts: EguiContexts,
    mut options: ResMut<GizmoOptions>,
    mut auto_scale: ResMut<GizmoAutoScale>,
) {
    let over_ui = contexts
        .ctx_mut()
        .map(|c| c.wants_pointer_input())
        .unwrap_or(false);

    let mut enabled: EnumSet<GizmoMode> = EnumSet::empty();
    if clock.paused && !over_ui {
        if modes.translate {
            enabled |= GizmoMode::all_translate();
        }
        if modes.rotate {
            enabled |= GizmoMode::all_rotate();
        }
        if modes.scale {
            enabled |= GizmoMode::all_scale();
        }
    }
    options.gizmo_modes = enabled;

    // The user-facing GizmoScale slider feeds straight into the
    // auto-scaler's `object_fraction` so dragging the slider grows /
    // shrinks the gizmo regardless of zoom.
    let defaults = GizmoAutoScale::default();
    auto_scale.object_fraction = defaults.object_fraction * scale.0;
}

fn manage_proxy_target(
    selection: Res<Selection>,
    options: Res<GizmoOptions>,
    mut commands: Commands,
    proxies: Query<(Entity, Option<&GizmoTarget>), With<GizmoProxy>>,
) {
    let Ok((entity, has_target)) = proxies.single() else {
        return;
    };
    let want_target = selection.vehicle.is_some() && !options.gizmo_modes.is_empty();
    match (want_target, has_target.is_some()) {
        (true, false) => {
            commands.entity(entity).insert(GizmoTarget::default());
        }
        (false, true) => {
            commands.entity(entity).remove::<GizmoTarget>();
        }
        _ => {}
    }
}

/// Mirror the selected vehicle's chassis half-extents onto the proxy
/// as an `Aabb` so [`auto_scale_gizmo_to_target`] can match the
/// gizmo to the *real* vehicle size instead of the fallback.
fn sync_proxy_aabb(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    mut commands: Commands,
    proxies: Query<Entity, With<GizmoProxy>>,
) {
    let Ok(entity) = proxies.single() else { return };
    let Some(state) = selection.vehicle.and_then(|id| sim.0.vehicle(id)) else {
        commands.entity(entity).remove::<Aabb>();
        return;
    };
    let s = state.spec.chassis.size;
    let half = bevy::math::Vec3A::new(
        (s.x as f32) * 0.5,
        (s.y as f32) * 0.5,
        (s.z as f32) * 0.5,
    );
    commands.entity(entity).insert(Aabb {
        center: bevy::math::Vec3A::ZERO,
        half_extents: half,
    });
}

/// Copy the sim's selected-vehicle pose into the proxy's `Transform`
/// whenever the gizmo isn't being actively dragged. While the gizmo
/// is active, the upstream plugin owns the `Transform` and we must
/// not stomp on its in-progress edit.
fn pull_proxy_from_sim(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    mut proxies: Query<(&mut Transform, Option<&GizmoTarget>), With<GizmoProxy>>,
) {
    let Ok((mut transform, target)) = proxies.single_mut() else {
        return;
    };
    if target.map(|t| t.is_active()).unwrap_or(false) {
        return;
    }
    let Some(id) = selection.vehicle else { return };
    if sim.0.vehicle(id).is_none() {
        return;
    }
    let pose = sim.0.vehicle_pose(id);
    transform.translation = Vec3::new(
        pose.point.x as f32,
        pose.point.y as f32,
        pose.point.z as f32,
    );
    transform.rotation = Quat::from_xyzw(
        pose.rotation.x as f32,
        pose.rotation.y as f32,
        pose.rotation.z as f32,
        pose.rotation.w as f32,
    );
    transform.scale = Vec3::ONE;
}

/// While the gizmo is active, write the proxy's edited `Transform`
/// back to the sim's vehicle pose.
fn push_proxy_to_sim(
    selection: Res<Selection>,
    mut sim: ResMut<GearboxSim>,
    proxies: Query<(&Transform, &GizmoTarget), With<GizmoProxy>>,
) {
    let Ok((transform, target)) = proxies.single() else {
        return;
    };
    if !target.is_active() {
        return;
    }
    let Some(id) = selection.vehicle else { return };
    let pose = Pose {
        point: Point::new(
            transform.translation.x as f64,
            transform.translation.y as f64,
            transform.translation.z as f64,
        ),
        rotation: Quaternion::new(
            transform.rotation.w as f64,
            transform.rotation.x as f64,
            transform.rotation.y as f64,
            transform.rotation.z as f64,
        ),
    };
    sim.0.set_vehicle_pose(id, pose);
}

