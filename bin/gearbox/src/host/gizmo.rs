//! The pose gizmo: while physics is paused the selected object gets every
//! translate handle and rotate ring of `transform-gizmo`, as the viewer had
//! before the mara re-host, on the machine's body (or the object's origin);
//! a drag moves and turns the whole object with `PoseRoot`.

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

/// Where the handles sit: a machine's body, which physics moves away from
/// the spawn point, else the object's own origin.
fn pivot_of(world: &mut World, root: Entity) -> Option<Transform> {
    let body = world
        .resource::<ControllerInventory>()
        .machines
        .iter()
        .find(|m| m.scene_root == Some(root))
        .and_then(|m| m.body.clone());
    if let Some(body) = body {
        let mut q = world.query::<(Entity, &UsdPrimRef, &GlobalTransform)>();
        let found: Vec<(Entity, GlobalTransform)> = q
            .iter(world)
            .filter(|(_, prim, _)| prim.path == body.as_str())
            .map(|(e, _, gt)| (e, *gt))
            .collect();
        if let Some((_, gt)) = found.into_iter().find(|(e, _)| is_under(world, *e, root)) {
            let (_, rotation, translation) = gt.to_scale_rotation_translation();
            return Some(Transform { translation, rotation, scale: Vec3::ONE });
        }
    }
    world.get::<Transform>(root).map(|t| Transform { scale: Vec3::ONE, ..*t })
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
        let Some(root) = world.resource::<Selection>().0 else {
            return false;
        };
        let Some(root_pose) = world
            .get::<LoadedAsset>(root)
            .and_then(|_| world.get::<Transform>(root))
            .copied()
        else {
            return false;
        };
        let Some(pivot) = pivot_of(world, root) else {
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
            // Carry the whole object by the pivot's change: turn about the
            // pivot, then move with it.
            let turn = new_pivot.rotation * pivot.rotation.inverse();
            let translation = new_pivot.translation + turn * (root_pose.translation - pivot.translation);
            let rotation = (turn * root_pose.rotation).normalize();
            world
                .resource_mut::<HostCommands>()
                .0
                .push(HostCommand::PoseRoot { root, translation, rotation });
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
