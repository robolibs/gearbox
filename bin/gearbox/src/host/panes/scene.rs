//! Scene: every loaded object with visibility, remove and select, loading
//! and reloading, and the selected object's pose, editable while paused.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::{SeparatorStyle, Tab, TabContainer};
use mara_core::pod::{Pod, PodResponse};
use mara_core::vocab::Id as MaraId;
use std::collections::HashMap;

use super::{PaneCtx, button_clicked, cid, pick_usd_file, pid, pod_response, short_path};
use crate::controller::ControllerInventory;
use crate::host::PANE_SCENE as P;
use crate::load::LoadedAsset;
use crate::viewer::commands::HostCommand;
use crate::viewer::systems::Selection;

const POSE_RANGE_M: f64 = 100_000.0;

struct Object {
    entity: Entity,
    label: String,
    machine: bool,
    visible: bool,
}

/// Every loaded object, in the order the list shows them.
fn objects(world: &mut World) -> Vec<Object> {
    let machines: Vec<Entity> = world
        .resource::<ControllerInventory>()
        .machines
        .iter()
        .filter_map(|m| m.scene_root)
        .collect();
    let mut q = world.query::<(Entity, &LoadedAsset, Option<&Visibility>)>();
    let mut objects: Vec<Object> = q
        .iter(world)
        .map(|(entity, asset, vis)| Object {
            entity,
            label: asset.label.clone(),
            machine: machines.contains(&entity),
            visible: !matches!(vis, Some(Visibility::Hidden)),
        })
        .collect();
    objects.sort_by(|a, b| (&a.label, a.entity).cmp(&(&b.label, b.entity)));
    objects
}

pub fn tab(world: &mut World, ctx: &PaneCtx) -> Tab {
    let accent = ctx.accent;
    let selection = world.resource::<Selection>().0;
    let paused = !world.resource::<gearbox_api::PhysicsActive>().0;
    let objects = objects(world);
    let count = objects.len();

    // Objects: one row each, sized to the rows rather than the shelf.
    let list_pod = pid(P, "objects", 1);
    let list = if objects.is_empty() {
        Pod::new(list_pod).with_readout("objects", "nothing loaded")
    } else {
        ctx.sync_list_memory(
            list_pod,
            selection.and_then(|s| objects.iter().position(|o| o.entity == s)),
            None,
        );
        let labels: Vec<String> = objects.iter().map(|o| o.label.clone()).collect();
        let trailing: Vec<String> = objects
            .iter()
            .map(|o| {
                let kind = if o.machine { "machine" } else { "object" };
                if o.visible { kind.to_string() } else { format!("{kind} · hidden") }
            })
            .collect();
        Pod::new(list_pod)
            .with_separator(SeparatorStyle::LineDots)
            .resizable()
            .with_hybrid_select_list(labels, Some(trailing), accent)
    };
    let objects_pods = vec![
        Pod::new(pid(P, "objects", 0))
            .with_button("Load USD…", accent)
            .with_button("Reload", accent),
        list,
    ];
    let selected_visible = selection
        .and_then(|s| objects.iter().find(|o| o.entity == s))
        .is_none_or(|o| o.visible);

    // Selected object: where it is, and drag values for its pose while paused.
    let picked = selection.and_then(|root| {
        let entity = world.get_entity(root).ok()?;
        let asset = entity.get::<LoadedAsset>()?;
        let transform = entity.get::<Transform>()?;
        let world_pos = entity
            .get::<GlobalTransform>()
            .map(|gt| gt.translation())
            .unwrap_or(transform.translation);
        let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
        Some((
            root,
            asset.label.clone(),
            asset.path.display().to_string(),
            world_pos,
            transform.translation,
            yaw,
            transform.rotation,
        ))
    });
    let pose_pod = pid(P, "selected", 0);
    let selected_pod = match &picked {
        Some((_, label, path, world_pos, local, yaw, _)) => {
            let mut pod = Pod::new(pose_pod)
                .with_readout("object", label.clone())
                .with_readout("file", short_path(path, 34))
                .with_readout(
                    "position",
                    format!("{:+.2}, {:+.2}, {:+.2}", world_pos.x, world_pos.y, world_pos.z),
                );
            if paused {
                let clamp = |v: f32| (v as f64).clamp(-POSE_RANGE_M, POSE_RANGE_M);
                let range = -POSE_RANGE_M..=POSE_RANGE_M;
                pod = pod
                    .with_drag_value("X", clamp(local.x), 0.05, range.clone(), 3, " m")
                    .with_drag_value("Y", clamp(local.y), 0.05, range.clone(), 3, " m")
                    .with_drag_value("Z", clamp(local.z), 0.05, range, 3, " m")
                    .with_drag_value(
                        "yaw",
                        (yaw.to_degrees() as f64).clamp(-180.0, 180.0),
                        0.5,
                        -180.0..=180.0,
                        1,
                        "°",
                    );
            } else {
                pod = pod.with_readout("pose", "pause physics to edit");
            }
            ctx.sync_toggles(pose_pod, &[selected_visible]);
            pod.with_toggle_initial("visible", accent, selected_visible)
                .with_button("Remove from scene", accent)
        }
        None => Pod::new(pose_pod)
            .with_readout("object", "none")
            .with_readout("hint", "click an object here or in the view"),
    };
    // The pose container's pod still answers at index 2, after the two
    // object pods.
    Tab::new(cid(P, "scene"), format!("Scene ({count})"), "list")
        .pods(objects_pods)
        .containers(vec![TabContainer::new(
            cid(P, "selected"),
            "Selected",
            "cursor-click",
            vec![selected_pod],
        )])
}

pub fn apply(responses: &HashMap<MaraId, Vec<PodResponse>>, world: &mut World, ctx: &PaneCtx) {
    let scene_id = cid(P, "scene");
    let selected_pod_idx = 2;
    let paused = !world.resource::<gearbox_api::PhysicsActive>().0;
    let selection = world.resource::<Selection>().0;
    if button_clicked(responses, scene_id, 0, 0)
        && let Some(path) = pick_usd_file()
    {
        ctx.send(HostCommand::Load(path));
    }
    if button_clicked(responses, scene_id, 0, 1) {
        ctx.send(HostCommand::Reload);
    }
    if let Some(rows) = pod_response(responses, scene_id, 1).and_then(|r| r.hybrid_select_lists.first()) {
        let objects = objects(world);
        if let Some(object) = rows.body_clicked.and_then(|i| objects.get(i)) {
            ctx.send(HostCommand::SelectRoot(Some(object.entity)));
        }
        if let Some(object) = rows.body_double_clicked.and_then(|i| objects.get(i)) {
            ctx.send(if object.machine {
                HostCommand::FlyToMachine(object.entity)
            } else {
                HostCommand::FitPrim(object.entity)
            });
        }
    }
    if let (Some(root), Some(resp)) = (selection, pod_response(responses, scene_id, selected_pod_idx)) {
        if let Some(toggle) = resp.toggles.first().filter(|t| t.changed) {
            ctx.send(HostCommand::SetVisibility(root, toggle.on));
        }
        if resp.buttons.first().is_some_and(|b| b.clicked) {
            ctx.send(HostCommand::Despawn(root));
        }
    }
    let picked = selection.and_then(|root| {
        let entity = world.get_entity(root).ok()?;
        let transform = entity.get::<Transform>()?;
        let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
        Some((root, transform.translation, yaw, transform.rotation))
    });
    if paused
        && let Some((root, local, yaw, rotation)) = picked
        && let Some(resp) = pod_response(responses, scene_id, selected_pod_idx)
        && resp.drag_values.iter().any(|d| d.changed)
    {
        let value = |i: usize, fallback: f32| {
            resp.drag_values.get(i).map(|d| d.value as f32).unwrap_or(fallback)
        };
        ctx.send(HostCommand::PoseRoot {
            root,
            translation: Vec3::new(value(0, local.x), value(1, local.y), value(2, local.z)),
            // A new heading keeps whatever tilt the gizmo gave the object.
            rotation: {
                let (_, pitch, roll) = rotation.to_euler(EulerRot::YXZ);
                Quat::from_euler(EulerRot::YXZ, value(3, yaw.to_degrees()).to_radians(), pitch, roll)
            },
        });
    }
}
