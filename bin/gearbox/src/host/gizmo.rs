//! The pose gizmo: while physics is paused the selected object gets every
//! translate handle and rotate ring of `transform-gizmo`, as the viewer had
//! before the mara re-host, on the machine's body (or the object's origin);
//! a drag moves and turns the whole object, or only the selected machine when
//! others share its stage, with `Pose`.

use bevy::prelude::*;
use mara::ui::modules::bevy as mara_bevy;
use transform_gizmo::math::Transform as GizmoTransform;
use transform_gizmo::{
    Gizmo as GizmoCore, GizmoConfig as GizmoCoreConfig, GizmoInteraction, GizmoMode,
    GizmoOrientation, GizmoVisuals, Rect as GizmoRect, mint,
};
use usd_bevy::UsdPrimRef;

use crate::controller::ControllerInventory;
use crate::load::LoadedAsset;
use crate::viewer::commands::{HostCommand, HostCommands};
use crate::viewer::systems::{GizmoGrab, Selection, SelectionRing};

/// The handles follow the object's apparent size, 80 % of it, within these
/// pixel bounds.
const SIZE_SHARE: f32 = 0.8;
const MIN_SIZE_PX: f32 = 6.0;
const MAX_SIZE_PX: f32 = 200.0;
/// Far plane of the gizmo's own projection. Bevy's camera projection is
/// reversed-Z with no far plane, which the gizmo cannot unproject.
const GIZMO_FAR_M: f32 = 10_000.0;

#[derive(Default)]
pub struct PoseGizmo {
    gizmo: GizmoCore,
    viewport: Option<egui::Rect>,
    shown: bool,
}

fn rows(m: Mat4) -> mint::RowMatrix4<f64> {
    let r = |i: usize| {
        let v = m.row(i);
        mint::Vector4 { x: v.x as f64, y: v.y as f64, z: v.z as f64, w: v.w as f64 }
    };
    mint::RowMatrix4 { x: r(0), y: r(1), z: r(2), w: r(3) }
}

fn to_gizmo(t: &Transform) -> GizmoTransform {
    let (p, q) = (t.translation, t.rotation);
    GizmoTransform {
        scale: mint::Vector3 { x: 1.0, y: 1.0, z: 1.0 },
        rotation: mint::Quaternion {
            v: mint::Vector3 { x: q.x as f64, y: q.y as f64, z: q.z as f64 },
            s: q.w as f64,
        },
        translation: mint::Vector3 { x: p.x as f64, y: p.y as f64, z: p.z as f64 },
    }
}

fn from_gizmo(t: &GizmoTransform) -> Transform {
    Transform {
        translation: Vec3::new(t.translation.x as f32, t.translation.y as f32, t.translation.z as f32),
        rotation: Quat::from_xyzw(
            t.rotation.v.x as f32,
            t.rotation.v.y as f32,
            t.rotation.v.z as f32,
            t.rotation.s as f32,
        ),
        scale: Vec3::ONE,
    }
}

fn is_under(world: &World, mut entity: Entity, root: Entity) -> bool {
    for _ in 0..64 {
        if entity == root {
            return true;
        }
        match world.get::<ChildOf>(entity) {
            Some(parent) => entity = parent.parent(),
            None => return false,
        }
    }
    false
}

fn rigid(gt: &GlobalTransform) -> Transform {
    let (_, rotation, translation) = gt.to_scale_rotation_translation();
    Transform { translation, rotation, scale: Vec3::ONE }
}

/// Where the handles sit: the selected machine's body, which physics moves
/// away from the spawn point, else the origin of what a drag carries.
fn pivot_of(world: &mut World, root: Entity, carried: Entity) -> Option<Transform> {
    let body = world
        .resource::<Selection>()
        .machine(world.resource::<ControllerInventory>())
        .and_then(|m| m.body.clone());
    if let Some(body) = body {
        let mut q = world.query::<(Entity, &UsdPrimRef, &GlobalTransform)>();
        let found: Vec<(Entity, GlobalTransform)> = q
            .iter(world)
            .filter(|(_, prim, _)| prim.path == body.as_str())
            .map(|(e, _, gt)| (e, *gt))
            .collect();
        if let Some((_, gt)) = found.into_iter().find(|(e, _)| is_under(world, *e, root)) {
            return Some(rigid(&gt));
        }
    }
    if carried != root {
        return world.get::<GlobalTransform>(carried).map(rigid);
    }
    world.get::<Transform>(root).map(|t| Transform { scale: Vec3::ONE, ..*t })
}

/// What a drag carries: the selected machine's own prim, with its parent's
/// world pose, when other machines share its stage; else the whole object.
/// `None` when that machine's prim is not in the scene.
fn carried(world: &mut World, root: Entity) -> Option<(Entity, Option<GlobalTransform>)> {
    let inventory = world.resource::<ControllerInventory>();
    let Some(prim) = world
        .resource::<Selection>()
        .machine(inventory)
        .filter(|_| inventory.only(root).is_none())
        .map(|m| m.prim_path.clone())
    else {
        return Some((root, None));
    };
    let mut q = world.query::<(Entity, &UsdPrimRef)>();
    let found: Vec<Entity> = q
        .iter(world)
        .filter(|(_, p)| p.path == prim)
        .map(|(e, _)| e)
        .collect();
    let own = found.into_iter().find(|e| is_under(world, *e, root))?;
    let parent = world.get::<ChildOf>(own)?.parent();
    Some((own, Some(*world.get::<GlobalTransform>(parent)?)))
}

/// Where a drag that took the handles from `pivot` to `moved` puts `pose`,
/// held in `frame` or, without one, in world space: turned about the pivot,
/// then moved with it.
fn dragged(
    pose: Transform,
    frame: Option<GlobalTransform>,
    pivot: Transform,
    moved: Transform,
) -> Transform {
    let held = frame.map_or(pose, |f| f.mul_transform(pose).compute_transform());
    let turn = moved.rotation * pivot.rotation.inverse();
    let carried = Transform {
        translation: moved.translation + turn * (held.translation - pivot.translation),
        rotation: (turn * held.rotation).normalize(),
        scale: held.scale,
    };
    frame.map_or(carried, |f| {
        GlobalTransform::from(carried).reparented_to(&f)
    })
}

impl PoseGizmo {
    /// Feed this frame's pointer to the gizmo before the viewport ticks, so
    /// a grabbed handle keeps the camera from orbiting and the click from
    /// reselecting.
    pub fn interact(&mut self, egui: &egui::Context, world: &mut World) {
        self.shown = false;
        let grabbed = self.update(egui, world);
        if let Some(mut grab) = world.get_resource_mut::<GizmoGrab>() {
            grab.0 = grabbed;
        }
    }

    fn update(&mut self, egui: &egui::Context, world: &mut World) -> bool {
        let Some(viewport) = self.viewport else {
            return false;
        };
        if world.resource::<gearbox_api::PhysicsActive>().0 {
            return false;
        }
        let Some(root) = world.resource::<Selection>().root() else {
            return false;
        };
        if world.get::<LoadedAsset>(root).is_none() {
            return false;
        }
        let Some((target, frame)) = carried(world, root) else {
            return false;
        };
        let Some(pose) = world.get::<Transform>(target).copied() else {
            return false;
        };
        let Some(pivot) = pivot_of(world, root, target) else {
            return false;
        };
        let camera = {
            let mut q = world
                .query_filtered::<(&GlobalTransform, &Projection), With<mara_bevy::ChaseCamera>>();
            q.iter(world).next().map(|(gt, p)| (*gt, p.clone()))
        };
        let Some((cam_gt, projection)) = camera else {
            return false;
        };
        let (fov, near) = match projection {
            Projection::Perspective(p) => (p.fov, p.near),
            _ => (0.8, 0.1),
        };
        let aspect = viewport.width() / viewport.height().max(1.0);
        let clip_from_view = Mat4::perspective_rh(fov, aspect, near.max(0.01), GIZMO_FAR_M);

        // Size from the object's projected radius.
        let radius = world.resource::<SelectionRing>().outer_radius.max(0.1);
        let distance = cam_gt.translation().distance(pivot.translation).max(0.1);
        let projected = radius / (distance * (fov * 0.5).tan()) * (viewport.height() * 0.5);
        let visuals = GizmoVisuals {
            gizmo_size: (projected * SIZE_SHARE).clamp(MIN_SIZE_PX, MAX_SIZE_PX),
            ..Default::default()
        };
        self.gizmo.update_config(GizmoCoreConfig {
            view_matrix: rows(Mat4::from(cam_gt.affine().inverse())),
            projection_matrix: rows(clip_from_view),
            viewport: GizmoRect::from_x_y_ranges(
                viewport.min.x..=viewport.max.x,
                viewport.min.y..=viewport.max.y,
            ),
            modes: GizmoMode::all_translate() | GizmoMode::all_rotate(),
            orientation: GizmoOrientation::Global,
            visuals,
            pixels_per_point: egui.pixels_per_point(),
            ..Default::default()
        });

        let (pos, pressed, down) = egui.input(|i| {
            (i.pointer.latest_pos(), i.pointer.primary_pressed(), i.pointer.primary_down())
        });
        let over_viewport =
            pos.is_some_and(|p| viewport.contains(p)) && !egui.is_pointer_over_egui();
        let interaction = GizmoInteraction {
            cursor_pos: pos.map(|p| (p.x, p.y)).unwrap_or((-1.0, -1.0)),
            hovered: over_viewport,
            drag_started: pressed && over_viewport,
            dragging: down,
        };
        self.shown = true;
        if let Some((_, moved)) = self.gizmo.update(interaction, &[to_gizmo(&pivot)])
            && let Some(new_pivot) = moved.first().map(from_gizmo)
            && new_pivot.translation.is_finite()
        {
            let placed = dragged(pose, frame, pivot, new_pivot);
            world
                .resource_mut::<HostCommands>()
                .0
                .push(HostCommand::Pose {
                    entity: target,
                    translation: placed.translation,
                    rotation: placed.rotation,
                });
        }
        self.gizmo.is_focused()
    }

    /// Remember where the viewport is and paint the handles over it.
    pub fn paint(&mut self, egui: &egui::Context, viewport: mara::ui::mara_core::vocab::Rect) {
        let rect: egui::Rect = viewport.into();
        self.viewport = Some(rect);
        if !self.shown {
            return;
        }
        let data = self.gizmo.draw();
        if data.vertices.is_empty() {
            return;
        }
        let mut mesh = egui::Mesh::default();
        for (v, c) in data.vertices.iter().zip(&data.colors) {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: egui::pos2(v[0], v[1]),
                uv: egui::epaint::WHITE_UV,
                color: egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]).into(),
            });
        }
        mesh.indices = data.indices;
        egui.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("gearbox_pose_gizmo"),
        ))
        .with_clip_rect(rect)
        .add(egui::Shape::mesh(mesh));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::MachineRef;
    use crate::viewer::systems::Selected;

    fn selecting(world: &mut World, selected: Selected) {
        world.insert_resource(Selection(Some(selected)));
    }

    fn machine(stage: Entity, id: &str) -> Selected {
        Selected::Machine(MachineRef {
            scene_root: stage,
            machine_id: id.into(),
        })
    }

    #[test]
    fn a_drag_carries_only_the_selected_machine_of_a_shared_stage() {
        let mut world = World::new();
        let (stage, bodies) = crate::controller::spawn_fleet(&mut world, &["Eve", "Percy"]);
        let frame = GlobalTransform::from_translation(Vec3::X * 4.0);
        world.entity_mut(stage).insert(frame);
        let percy = world.get::<ChildOf>(bodies[1]).unwrap().parent();
        selecting(&mut world, machine(stage, "percy"));
        assert_eq!(carried(&mut world, stage), Some((percy, Some(frame))));
        selecting(&mut world, Selected::Asset(stage));
        assert_eq!(carried(&mut world, stage), Some((stage, None)));
    }

    #[test]
    fn a_drag_carries_a_lone_machine_with_its_whole_stage() {
        let mut world = World::new();
        let (stage, _) = crate::controller::spawn_fleet(&mut world, &["Solo"]);
        world.entity_mut(stage).insert(GlobalTransform::IDENTITY);
        selecting(&mut world, machine(stage, "solo"));
        assert_eq!(carried(&mut world, stage), Some((stage, None)));
    }

    #[test]
    fn a_dragged_prim_lands_where_the_handles_took_it() {
        let frame = GlobalTransform::from(Transform {
            translation: Vec3::new(10.0, 0.0, -2.0),
            rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        });
        let pose = Transform::from_xyz(1.0, 2.0, 0.0);
        let held = frame.mul_transform(pose).compute_transform();
        let pivot = Transform::from_translation(held.translation + Vec3::Y);
        let moved = Transform {
            translation: pivot.translation + Vec3::Z * 5.0,
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        };
        let placed = frame
            .mul_transform(dragged(pose, Some(frame), pivot, moved))
            .compute_transform();
        let expected = moved.translation + moved.rotation * (held.translation - pivot.translation);
        assert!(placed.translation.distance(expected) < 1.0e-4, "{placed:?}");
        for axis in [Vec3::X, Vec3::Y] {
            let turned = moved.rotation * held.rotation * axis;
            assert!(
                (placed.rotation * axis).distance(turned) < 1.0e-4,
                "{placed:?}"
            );
        }
        let root = dragged(pose, None, pivot, moved);
        assert_eq!(
            root.translation,
            moved.translation + moved.rotation * (pose.translation - pivot.translation)
        );
    }
}
