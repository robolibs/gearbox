//! Selection: the active stage, its actions, the selected asset (with pose
//! editing while paused) and the selected prim.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use usd_bevy::UsdPrimRef;
use usd_bevy::route::meta::UsdKind;
use usd_bevy::route::{audio::UsdSpatialAudio, coverage::UsdProcedural};

use super::{PaneCtx, button_clicked, cid, nonempty_or, pick_usd_file, pid, pod_response};
use crate::host::PANE_SELECTION as P;
use crate::load::LoadedAsset;
use crate::viewer::commands::HostCommand;
use crate::viewer::state::{SelectedPrim, StageInfo};
use crate::viewer::systems::Selection;

const POSE_RANGE_M: f64 = 100_000.0;

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let accent = ctx.accent;
    let info = world.resource::<StageInfo>().clone();
    let selection = world.resource::<Selection>().0;
    let selected = world.resource::<SelectedPrim>().0;
    let paused = !world.resource::<gearbox_api::PhysicsActive>().0;

    body.add_normal(
        cid(P, "stage"),
        "Active stage",
        "folder",
        vec![
            Pod::new(pid(P, "stage", 0))
                .with_readout("file", nonempty_or(&info.path, "No active stage"))
                .with_readout(
                    "default prim",
                    info.default_prim.as_deref().unwrap_or("None"),
                )
                .with_readout("layers", info.layer_count.to_string())
                .with_readout("variants", info.variant_count.to_string()),
        ],
    );
    let actions_id = cid(P, "actions");
    body.add_normal(
        actions_id,
        "Stage actions",
        "play",
        vec![
            Pod::new(pid(P, "actions", 0))
                .with_button("Add USD…", accent)
                .with_button("Reveal active file", accent)
                .with_button("Reload active stage", accent)
                .with_button("Clear prim selection", accent),
        ],
    );

    // Selected asset: readouts, and drag values for its pose while paused.
    let asset = selection.and_then(|root| {
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
    let selected_pod = match &asset {
        Some((_, label, path, world_pos, local, yaw)) => {
            let mut pod = Pod::new(pose_pod)
                .with_readout("asset", label.clone())
                .with_readout("path", path.clone())
                .with_readout(
                    "position",
                    format!(
                        "{:+.2}, {:+.2}, {:+.2}",
                        world_pos.x, world_pos.y, world_pos.z
                    ),
                );
            if paused {
                let clamp = |v: f32| (v as f64).clamp(-POSE_RANGE_M, POSE_RANGE_M);
                pod = pod
                    .with_drag_value("X", clamp(local.x), 0.05, -POSE_RANGE_M..=POSE_RANGE_M, 3, " m")
                    .with_drag_value("Y", clamp(local.y), 0.05, -POSE_RANGE_M..=POSE_RANGE_M, 3, " m")
                    .with_drag_value("Z", clamp(local.z), 0.05, -POSE_RANGE_M..=POSE_RANGE_M, 3, " m")
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
            .with_readout("asset", "None")
            .with_readout("hint", "Click a loaded asset or pick one in the outliner"),
    };
    body.add_normal(selected_id, "Selected asset", "cube", vec![selected_pod]);

    // Selected prim: name, path, what it carries.
    let prim = selected.and_then(|entity| {
        let e = world.get_entity(entity).ok()?;
        let pr = e.get::<UsdPrimRef>()?;
        let name = e
            .get::<Name>()
            .map(|n| n.as_str().to_string())
            .unwrap_or_default();
        let mut tags: Vec<String> = Vec::new();
        if e.contains::<Mesh3d>() {
            tags.push("mesh".into());
        }
        if let Some(kind) = e.get::<UsdKind>() {
            tags.push(format!("kind:{}", kind.kind));
        }
        if e.get::<Children>().is_some_and(|c| !c.is_empty()) {
            tags.push("parent".into());
        }
        if e.contains::<UsdSpatialAudio>() {
            tags.push("audio".into());
        }
        if e.contains::<UsdProcedural>() {
            tags.push("procedural".into());
        }
        if matches!(e.get::<Visibility>(), Some(Visibility::Hidden)) {
            tags.push("hidden".into());
        }
        Some((name, pr.path.clone(), tags))
    });
    let prim_id = cid(P, "prim");
    let prim_pod = match prim {
        Some((name, path, tags)) => {
            let mut pod = Pod::new(pid(P, "prim", 0))
                .with_readout("name", name)
                .with_readout("path", path);
            if !tags.is_empty() {
                pod = pod.with_badge_row("carries", tags, accent);
            }
            pod
        }
        None => Pod::new(pid(P, "prim", 0)).with_readout("prim", "Click a prim in the outliner"),
    };
    body.add_normal(prim_id, "Selected prim", "cube-tree", vec![prim_pod]);

    let responses = body.render();
    if button_clicked(&responses, actions_id, 0, 0)
        && let Some(path) = pick_usd_file()
    {
        ctx.send(HostCommand::Load(path));
    }
    if button_clicked(&responses, actions_id, 0, 1) && !info.path.is_empty() {
        let p = std::path::Path::new(&info.path);
        let target = p.parent().unwrap_or(p);
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
    if button_clicked(&responses, actions_id, 0, 2) {
        ctx.send(HostCommand::Reload);
    }
    if button_clicked(&responses, actions_id, 0, 3) {
        ctx.send(HostCommand::SelectPrim(None));
    }
    if paused
        && let Some((root, _, _, _, local, yaw)) = asset
        && let Some(resp) = pod_response(&responses, selected_id, 0)
        && resp.drag_values.iter().any(|d| d.changed)
    {
        let value = |i: usize, fallback: f32| {
            resp.drag_values
                .get(i)
                .map(|d| d.value as f32)
                .unwrap_or(fallback)
        };
        ctx.send(HostCommand::PoseRoot {
            root,
            translation: Vec3::new(
                value(0, local.x),
                value(1, local.y),
                value(2, local.z),
            ),
            yaw: value(3, yaw.to_degrees()).to_radians(),
        });
    }
}
