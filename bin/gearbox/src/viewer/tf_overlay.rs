//! TF tree overlay, the RViz way: one RGB frame per link of every machine's
//! USD link tree, an arrow from each child frame to its parent, and the
//! link name at the frame. Three toggles in the Overlays panel, all off at
//! start.

use bevy::prelude::*;
use usd_bevy::UsdPrimRef;

use super::overlays::DisplayToggles;
use crate::controller::{ControllerInventory, MachineInstanceSpec, find_prim_entity};
use crate::links::LinkRole;

/// TF lines draw over geometry: a frame inside a body is still visible.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct TfGizmos;

pub struct TfOverlayPlugin;

impl Plugin for TfOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_gizmo_group::<TfGizmos>()
            .add_systems(Startup, configure_tf_gizmos)
            .add_systems(Update, draw_tf_gizmos)
            .init_resource::<TfLabels>()
            .add_systems(PostUpdate, publish_tf_labels);
    }
}

fn configure_tf_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<TfGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 2.5;
}

const LINK_COLOR: Color = Color::srgb(1.0, 0.85, 0.1);
const BASE_AXES_M: f32 = 0.6;
const MIN_AXES_M: f32 = 0.12;
const MAX_AXES_M: f32 = 0.5;
const AXES_PER_SPACING: f32 = 0.3;

/// Every link of a machine with its world transform and its parent's.
struct Frame<'a> {
    name: &'a str,
    role: LinkRole,
    transform: GlobalTransform,
    parent: Option<Vec3>,
}

fn frames_of<'a>(
    machine: &'a MachineInstanceSpec,
    prims: &Query<(Entity, &UsdPrimRef)>,
    parents: &Query<&ChildOf>,
    transforms: &Query<&GlobalTransform>,
) -> Vec<Frame<'a>> {
    let Some(scene_root) = machine.scene_root else {
        return Vec::new();
    };
    let world_of = |prim: &str| {
        find_prim_entity(scene_root, prim, prims, parents)
            .and_then(|e| transforms.get(e).ok())
            .copied()
    };
    machine
        .links
        .links
        .iter()
        .filter_map(|link| {
            let transform = world_of(&link.prim_path)?;
            let parent = link
                .parent
                .as_deref()
                .and_then(|p| machine.links.get(p))
                .and_then(|p| world_of(&p.prim_path))
                .map(|t| t.translation());
            Some(Frame {
                name: &link.name,
                role: link.role,
                transform,
                parent,
            })
        })
        .collect()
}

fn axes_length(frame: &Frame<'_>) -> f32 {
    if frame.role == LinkRole::Base {
        return BASE_AXES_M;
    }
    match frame.parent {
        Some(p) => (frame.transform.translation().distance(p) * AXES_PER_SPACING)
            .clamp(MIN_AXES_M, MAX_AXES_M),
        None => MAX_AXES_M,
    }
}

fn draw_tf_gizmos(
    toggles: Res<DisplayToggles>,
    inventory: Res<ControllerInventory>,
    prims: Query<(Entity, &UsdPrimRef)>,
    parents: Query<&ChildOf>,
    transforms: Query<&GlobalTransform>,
    mut gizmos: Gizmos<TfGizmos>,
) {
    if !toggles.show_tf_frames && !toggles.show_tf_links {
        return;
    }
    for machine in &inventory.machines {
        for frame in frames_of(machine, &prims, &parents, &transforms) {
            if toggles.show_tf_frames {
                gizmos.axes(frame.transform, axes_length(&frame));
            }
            if toggles.show_tf_links
                && let Some(parent) = frame.parent
            {
                gizmos.arrow(frame.transform.translation(), parent, LINK_COLOR);
            }
        }
    }
}

/// Link names with their position in the viewport, as a fraction of the
/// render target, for the host to paint over the viewport.
#[derive(Resource, Default, Debug, Clone)]
pub struct TfLabels(pub Vec<TfLabel>);

#[derive(Debug, Clone)]
pub struct TfLabel {
    pub name: String,
    pub at: [f32; 2],
}

fn publish_tf_labels(
    toggles: Res<DisplayToggles>,
    inventory: Res<ControllerInventory>,
    prims: Query<(Entity, &UsdPrimRef)>,
    parents: Query<&ChildOf>,
    transforms: Query<&GlobalTransform>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut labels: ResMut<TfLabels>,
) {
    labels.0.clear();
    if !toggles.show_tf_names {
        return;
    }
    let Some((camera, cam_tr)) = cameras.iter().find(|(c, _)| c.is_active) else {
        return;
    };
    let Some(size) = camera.logical_viewport_size() else {
        return;
    };
    for machine in &inventory.machines {
        for frame in frames_of(machine, &prims, &parents, &transforms) {
            let Ok(screen) = camera.world_to_viewport(cam_tr, frame.transform.translation()) else {
                continue;
            };
            labels.0.push(TfLabel {
                name: frame.name.to_string(),
                at: [screen.x / size.x, screen.y / size.y],
            });
        }
    }
}
