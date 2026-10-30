//! TF tree overlay, the RViz way: one RGB frame per link of every machine's
//! USD link tree, an arrow from each child frame to its parent, and the
//! link name at the frame. Three toggles in the Overlays panel, all off at
//! start.

use bevy::prelude::*;

use super::overlays::DisplayToggles;
use crate::controller::{ControllerInventory, MachineInstanceSpec};
use crate::links::LinkRole;
use crate::prim_index::PrimIndex;

/// TF lines draw over geometry: a frame inside a body is still visible.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct TfGizmos;

pub struct TfOverlayPlugin;

impl Plugin for TfOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_gizmo_group::<TfGizmos>()
            .add_systems(Startup, configure_tf_gizmos)
            .init_resource::<TfLabels>()
            .add_systems(
                PostUpdate,
                (draw_tf_gizmos, publish_tf_labels)
                    .after(bevy::transform::TransformSystems::Propagate),
            );
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
/// Seconds the TF tree takes to fade in or out.
const TF_FADE_S: f32 = 0.6;

/// Every link of a machine with its world transform and its parent's.
struct Frame<'a> {
    name: &'a str,
    role: LinkRole,
    transform: GlobalTransform,
    parent: Option<Vec3>,
}

fn frames_of<'a>(
    machine: &'a MachineInstanceSpec,
    index: &PrimIndex,
    transforms: &Query<&GlobalTransform>,
) -> Vec<Frame<'a>> {
    let Some(scene_root) = machine.scene_root else {
        return Vec::new();
    };
    let world_of = |prim: &str| {
        index
            .find(scene_root, prim)
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
    if frame.role == LinkRole::Wheel {
        return 0.45;
    }
    if frame.role == LinkRole::Steer {
        return 0.16;
    }
    if frame.role == LinkRole::Base {
        return BASE_AXES_M;
    }
    match frame.parent {
        Some(p) => (frame.transform.translation().distance(p) * AXES_PER_SPACING)
            .clamp(MIN_AXES_M, MAX_AXES_M),
        None => MAX_AXES_M,
    }
}

/// The machines the TF tree is drawn on.
fn drawn<'a>(
    inventory: &'a ControllerInventory,
    toggles: &'a DisplayToggles,
) -> impl Iterator<Item = &'a MachineInstanceSpec> {
    inventory
        .machines
        .iter()
        .filter(|m| toggles.tf_machines.is_empty() || toggles.tf_machines.contains(&m.id))
}

/// A frame's X, Y and Z axes as red, green and blue arrows.
fn axes(gizmos: &mut Gizmos<TfGizmos>, at: &GlobalTransform, length: f32, alpha: f32) {
    use bevy::color::palettes::basic::{BLUE, GREEN, RED};
    let iso = at.to_isometry();
    let start = iso.translation.into();
    for (axis, color) in [(Vec3::X, RED), (Vec3::Y, GREEN), (Vec3::Z, BLUE)] {
        gizmos.arrow(start, iso.transform_point(axis * length).into(), Color::from(color).with_alpha(alpha));
    }
}

fn draw_tf_gizmos(
    time: Res<Time>,
    toggles: Res<DisplayToggles>,
    inventory: Res<ControllerInventory>,
    index: Res<PrimIndex>,
    transforms: Query<&GlobalTransform>,
    mut gizmos: Gizmos<TfGizmos>,
    mut shown: Local<(f32, bool, bool)>,
) {
    let on = toggles.show_tf_frames || toggles.show_tf_links;
    if on {
        (shown.1, shown.2) = (toggles.show_tf_frames, toggles.show_tf_links);
    }
    let step = time.delta_secs() / TF_FADE_S;
    shown.0 = if on { (shown.0 + step).min(1.0) } else { (shown.0 - step).max(0.0) };
    let (fade, frames, links) = *shown;
    if fade <= 0.0 {
        return;
    }
    let alpha = fade * fade * (3.0 - 2.0 * fade);
    for machine in drawn(&inventory, &toggles) {
        for frame in frames_of(machine, &index, &transforms)
            .into_iter()
            .filter(|f| !toggles.tf_wheels_only || f.role == LinkRole::Wheel)
        {
            if frames {
                axes(&mut gizmos, &frame.transform, axes_length(&frame), alpha);
            }
            if links && let Some(parent) = frame.parent {
                gizmos.arrow(frame.transform.translation(), parent, LINK_COLOR.with_alpha(alpha));
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
    index: Res<PrimIndex>,
    transforms: Query<&GlobalTransform>,
    cameras: Query<(&Camera, &GlobalTransform), With<mara::ui::modules::bevy::ChaseCamera>>,
    mut labels: ResMut<TfLabels>,
) {
    labels.0.clear();
    if !toggles.show_tf_names {
        return;
    }
    let Ok((camera, cam_tr)) = cameras.single() else {
        return;
    };
    let Some(size) = camera.logical_viewport_size() else {
        return;
    };
    for machine in drawn(&inventory, &toggles) {
        for frame in frames_of(machine, &index, &transforms)
            .into_iter()
            .filter(|f| !toggles.tf_wheels_only || f.role == LinkRole::Wheel)
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(id: &str) -> MachineInstanceSpec {
        MachineInstanceSpec {
            tracks: Vec::new(),
            devices: Vec::new(),
            scene_root: None,
            asset_label: "fleet".into(),
            source_path: String::new(),
            prim_path: format!("/Fleet/{id}"),
            id: id.into(),
            kind: None,
            interface_version: None,
            id_policy: String::new(),
            body: None,
            sensors: Vec::new(),
            powered_wheel_joints: Vec::new(),
            passive_wheel_joints: Vec::new(),
            steering_joints: Vec::new(),
            brake_joints: Vec::new(),
            suspension_joints: Vec::new(),
            tool_joints: Vec::new(),
            controllers: Vec::new(),
            links: Default::default(),
            grants: Vec::new(),
            model: None,
        }
    }

    #[test]
    fn the_tree_is_drawn_on_the_machines_named_or_on_all_when_none_are() {
        let inventory = ControllerInventory { machines: ["picker", "washer", "digger"].map(machine).to_vec() };
        let mut toggles = DisplayToggles::default();
        let ids = |toggles: &DisplayToggles| drawn(&inventory, toggles).map(|m| m.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&toggles), ["picker", "washer", "digger"]);
        toggles.tf_machines = vec!["digger".into(), "picker".into()];
        assert_eq!(ids(&toggles), ["picker", "digger"]);
    }
}
