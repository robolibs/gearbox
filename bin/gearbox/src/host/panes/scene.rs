//! Scene: every loaded object with visibility, remove and select, loading
//! and reloading, and the selected object's pose, editable while paused.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::SeparatorStyle;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::widget::{TreeIconKind, TreeIconSlot};

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
    icon: &'static str,
    machine: bool,
    visible: bool,
}

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let accent = ctx.accent;
    let selection = world.resource::<Selection>().0;
    let paused = !world.resource::<gearbox_api::PhysicsActive>().0;
    let machines: Vec<Entity> = world
        .resource::<ControllerInventory>()
        .machines
        .iter()
        .filter_map(|m| m.scene_root)
        .collect();
    let mut objects: Vec<Object> = {
        let mut q = world.query::<(Entity, &LoadedAsset, Option<&Visibility>)>();
        q.iter(world)
            .map(|(entity, asset, vis)| Object {
                entity,
                label: asset.label.clone(),
                icon: if machines.contains(&entity) { "vehicle-tractor" } else { "cube" },
                machine: machines.contains(&entity),
                visible: !matches!(vis, Some(Visibility::Hidden)),
            })
            .collect()
    };
    objects.sort_by(|a, b| a.label.cmp(&b.label));

    // Objects: one row each, eye and trash on the right.
    let objects_id = cid(P, "objects");
    let outbox = ctx.outbox.clone();
    let count = objects.len();
    body.add_normal(
        objects_id,
        format!("Objects ({count})"),
        "list",
        vec![
            Pod::new(pid(P, "objects", 0))
                .with_button("Load USD…", accent)
                .with_button("Reload", accent),
            Pod::new(pid(P, "objects", 1))
                .with_separator(SeparatorStyle::Line)
                .fill()
                .with_tree(16, move |tree| {
                    if objects.is_empty() {
                        let mut none = false;
                        let mut slot =
                            [TreeIconSlot::new(TreeIconKind::Glyph { on: "·", off: "·" }, &mut none)];
                        tree.row("empty", 0, None, None, "Nothing loaded", false, accent, &mut slot);
                        return;
                    }
                    for object in &objects {
                        let mut visible = object.visible;
                        let mut remove = false;
                        let mut slots = [
                            TreeIconSlot::new(TreeIconKind::Eye, &mut visible)
                                .with_tooltip("Toggle visibility"),
                            TreeIconSlot::new(TreeIconKind::Glyph { on: "🗑", off: "🗑" }, &mut remove)
                                .with_tooltip("Remove from scene"),
                        ];
                        let resp = tree.row(
                            object.entity.to_bits(),
                            0,
                            None,
                            Some(object.icon),
                            &object.label,
                            selection == Some(object.entity),
                            accent,
                            &mut slots,
                        );
                        if resp.body.clicked() {
                            outbox.push(HostCommand::SelectRoot(Some(object.entity)));
                        }
                        if resp.body.double_clicked() {
                            outbox.push(if object.machine {
                                HostCommand::FlyToMachine(object.entity)
                            } else {
                                HostCommand::FitPrim(object.entity)
                            });
                        }
                        if resp.icons.get(1).is_some_and(|i| i.clicked()) {
                            outbox.push(HostCommand::Despawn(object.entity));
                        }
                        if visible != object.visible {
                            outbox.push(HostCommand::SetVisibility(object.entity, visible));
                        }
                    }
                }),
        ],
    );

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
        ))
    });
    let selected_id = cid(P, "selected");
    let pose_pod = pid(P, "selected", 0);
    let selected_pod = match &picked {
        Some((_, label, path, world_pos, local, yaw)) => {
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
            pod
        }
        None => Pod::new(pose_pod)
            .with_readout("object", "none")
            .with_readout("hint", "click an object here or in the view"),
    };
    body.add_normal(selected_id, "Selected", "cube", vec![selected_pod]);

    let responses = body.render();
    if button_clicked(&responses, objects_id, 0, 0)
        && let Some(path) = pick_usd_file()
    {
        ctx.send(HostCommand::Load(path));
    }
    if button_clicked(&responses, objects_id, 0, 1) {
        ctx.send(HostCommand::Reload);
    }
    if paused
        && let Some((root, _, _, _, local, yaw)) = picked
        && let Some(resp) = pod_response(&responses, selected_id, 0)
        && resp.drag_values.iter().any(|d| d.changed)
    {
        let value = |i: usize, fallback: f32| {
            resp.drag_values.get(i).map(|d| d.value as f32).unwrap_or(fallback)
        };
        ctx.send(HostCommand::PoseRoot {
            root,
            translation: Vec3::new(value(0, local.x), value(1, local.y), value(2, local.z)),
            yaw: value(3, yaw.to_degrees()).to_radians(),
        });
    }
}
