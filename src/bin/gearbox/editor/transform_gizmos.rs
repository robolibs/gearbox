//! Transform gizmos — arrows / rings / stubs that follow the selected
//! vehicle. Tab cycles the three modes:
//!
//!   - [`GizmoMode::Translate`] → three thin axis-coloured arrows
//!   - [`GizmoMode::Rotate`]    → three thin axis-coloured rings
//!   - [`GizmoMode::Scale`]     → three axis-coloured stubs tipped
//!                                 with small cubes
//!
//! Real Bevy mesh entities (not immediate-mode) so there's no
//! z-fighting or flicker. Each handle owns its own material so we
//! can brighten it independently when the cursor hovers it — gives
//! a clear "you can grab this" cue.
//!
//! Hover detection uses a fattened bounding volume (cylinder for
//! axis handles, annulus for rings) so the user doesn't have to
//! pixel-hunt the thin visible mesh. Click-to-drag is a future
//! step — for now, precise edits live in the inspector's Transform
//! section.
//!
//! Local-space: handles rotate with the vehicle's chassis, so the
//! +X arrow always points out the vehicle's right side.
//!
//! Tab is ignored while egui has keyboard focus (typing in a field).

use std::f32::consts::FRAC_PI_2;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContexts;
use big_space::prelude::BigSpatialBundle;

use gearbox::{
    datapod::{Point, Pose, Quaternion},
    VehicleId,
};

use crate::viz::{GearboxSim, SimClock};
use crate::BigSpaceRoot;

use super::selection::Selection;
use super::style::{AXIS_X, AXIS_Y, AXIS_Z};

#[derive(Resource, Default, Copy, Clone, Debug, PartialEq, Eq)]
pub enum GizmoMode {
    #[default]
    Translate,
    Rotate,
    Scale,
}

impl GizmoMode {
    fn next(self) -> Self {
        match self {
            GizmoMode::Translate => GizmoMode::Rotate,
            GizmoMode::Rotate    => GizmoMode::Scale,
            GizmoMode::Scale     => GizmoMode::Translate,
        }
    }
}

/// What shape the handle is — used by the hover-pick system to
/// choose the right intersection test.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GizmoShape {
    /// Axis arrow / scale stub: line along the handle's local +Y.
    Axis,
    /// Rotation ring: annulus around the handle's local +Y axis.
    Ring,
}

/// Tag on the parent entity of a gizmo-mesh cluster. `local_axis` is
/// the handle's axis direction in the vehicle's local frame (before
/// the vehicle rotation is applied) — `Vec3::X`, `Y`, or `Z`.
#[derive(Component)]
pub struct GizmoHandle {
    pub mode: GizmoMode,
    pub local_axis: Vec3,
    pub shape: GizmoShape,
    pub material: Handle<StandardMaterial>,
    pub base_rgb: [f32; 3],
}

/// Which handle the cursor is currently over (at most one). `None`
/// when nothing's hovered.
#[derive(Resource, Default)]
pub struct HoveredGizmo(pub Option<Entity>);

/// User-controlled gizmo size multiplier. `1.0` is the built-in
/// default; bigger values make everything proportionally thicker AND
/// farther from the vehicle at the same time (because the whole
/// handle transform is scaled uniformly). Clamp range enforced by
/// the UI slider.
#[derive(Resource, Copy, Clone, Debug)]
pub struct GizmoScale(pub f32);

impl Default for GizmoScale {
    fn default() -> Self { Self(1.0) }
}

/// Shared mesh handles for the four gizmo primitives. Held in a
/// resource so the regen system can replace the mesh data in place
/// when the slider or the selected vehicle's reach changes.
#[derive(Resource)]
pub struct GizmoMeshHandles {
    pub shaft: Handle<Mesh>,
    pub tip:   Handle<Mesh>,
    pub ring:  Handle<Mesh>,
    pub cube:  Handle<Mesh>,
}

/// Which primitive a mesh entity represents. Used to update tip/cube
/// child Transform positions when the tip/cube sizes change.
#[derive(Component, Copy, Clone, PartialEq, Eq)]
pub enum GizmoMeshRole {
    Tip,
    Cube,
}

/// Active click-and-hold drag. Set when LMB is pressed on a hovered
/// handle; cleared on release. `pick_and_drag_system` checks this and
/// stays out of the way while it's `Some`.
#[derive(Resource, Default)]
pub struct GizmoDrag {
    pub active: Option<ActiveDrag>,
}

/// Per-axis state captured at press time — used to compute per-frame
/// deltas while the mouse is held.
pub struct ActiveDrag {
    pub mode: GizmoMode,
    pub shape: GizmoShape,
    pub vehicle: VehicleId,
    /// World-space axis the handle points along (unit length).
    pub world_axis: Vec3,
    /// Vehicle pose when the drag began — deltas are applied on top
    /// of this, not accumulated frame-over-frame.
    pub start_pose: Pose,
    /// For `Axis` shapes: parameter `t` along the axis at press.
    /// For `Ring` shapes: angle (radians) in the plane basis at press.
    pub start_param: f32,
    /// Plane basis for ring drags — so we can keep measuring angles
    /// in the same 2-D frame as the press.
    pub basis_e1: Vec3,
    pub basis_e2: Vec3,
}

// ── Mesh / material tuning ──────────────────────────────────────────
//
// The gizmo's *length* scales with the selected vehicle (shaft length,
// ring major radius), but its *thickness* is controlled purely by the
// user slider (`GizmoScale`) and stays reach-independent. We express
// lengths as unit-space fractions (parent transform will scale by
// `reach`), and thicknesses as world-space base sizes that get
// multiplied by the slider and divided by the current `reach` so the
// parent's uniform scale turns them back into the desired world size.

/// Shaft length in unit-space — final world length = `reach × this`.
const SHAFT_LEN: f32 = 0.80;
/// Rotation-ring major radius fraction — final world major radius =
/// `reach × this`.
const RING_MAJOR: f32 = 0.90;

// Base *world-space* sizes at slider = 1.0. These are the things the
// slider multiplies — they DO NOT scale with vehicle reach.
const BASE_SHAFT_RADIUS_W: f32 = 0.02;
const BASE_TIP_RADIUS_W:   f32 = 0.06;
const BASE_TIP_LEN_W:      f32 = 0.15;
const BASE_CUBE_EDGE_W:    f32 = 0.05;
/// Ring tube minor radius — matched to the shaft radius so rotation
/// rings and translate/scale segments share the same line-weight at
/// every slider value. (An earlier version made this 1.5× thicker to
/// compensate for the torus cross-section reading thinner than a
/// cylinder at very small sizes — the advantage inverts at larger
/// sizes, so now they track 1:1.)
const BASE_RING_MINOR_W:   f32 = BASE_SHAFT_RADIUS_W;

/// Baseline emissive strength. Kept modest so thin meshes don't bloom
/// into looking thick.
const EMISSIVE_BASE: f32 = 1.5;
/// Emissive boost on hover — clearly separates "you're looking at it"
/// from the idle state.
const EMISSIVE_HOVER: f32 = 5.0;

/// Axis-handle hover tolerance (unit-space radius around the axis
/// line). Much fatter than `SHAFT_RADIUS` so picks feel forgiving.
const AXIS_HIT_RADIUS: f32 = 0.08;
/// Ring hover tolerance (half-width of the annulus band).
const RING_HIT_BAND: f32 = 0.07;

// ────────────────────────────────────────────────────────────────────

/// Tab key cycles modes. Swallowed when egui has keyboard focus.
pub fn cycle_gizmo_mode(
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<GizmoMode>,
    mut contexts: EguiContexts,
) {
    let wants_kb = contexts
        .ctx_mut()
        .map(|c| c.wants_keyboard_input())
        .unwrap_or(false);
    if wants_kb {
        return;
    }
    if keys.just_pressed(KeyCode::Tab) {
        *mode = mode.next();
    }
}

pub fn setup_transform_gizmos(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    big_space_root: Res<BigSpaceRoot>,
) {
    // Placeholder meshes — real dimensions are written by
    // `regenerate_gizmo_meshes` on the first frame a vehicle is
    // selected. These four handles are shared across every axis
    // variant, so updating the asset propagates to all 9 handles.
    let shaft_mesh = meshes.add(Cylinder::new(0.01, SHAFT_LEN).mesh().resolution(16).build());
    let tip_mesh   = meshes.add(Cone::new(0.02, 0.05).mesh().resolution(16).build());
    let ring_mesh  = meshes.add(
        Torus::new(RING_MAJOR - 0.005, RING_MAJOR + 0.005)
            .mesh()
            .minor_resolution(8)
            .major_resolution(96)
            .build(),
    );
    let cube_mesh  = meshes.add(Cuboid::new(0.03, 0.03, 0.03));

    commands.insert_resource(GizmoMeshHandles {
        shaft: shaft_mesh.clone(),
        tip:   tip_mesh.clone(),
        ring:  ring_mesh.clone(),
        cube:  cube_mesh.clone(),
    });

    // Axis-aligning rotations: map the shape's natural +Y → target axis.
    let axes = [
        (AXIS_X, Vec3::X, Quat::from_rotation_z(-FRAC_PI_2)),
        (AXIS_Y, Vec3::Y, Quat::IDENTITY),
        (AXIS_Z, Vec3::Z, Quat::from_rotation_x( FRAC_PI_2)),
    ];
    let root = big_space_root.0;

    for (egui_color, local_axis, axis_rot) in axes {
        let rgb = egui_to_rgb(egui_color);
        spawn_arrow(
            &mut commands, &mut materials, root, local_axis, axis_rot,
            shaft_mesh.clone(), tip_mesh.clone(), rgb,
        );
        spawn_ring(
            &mut commands, &mut materials, root, local_axis, axis_rot,
            ring_mesh.clone(), rgb,
        );
        spawn_scale_stub(
            &mut commands, &mut materials, root, local_axis, axis_rot,
            shaft_mesh.clone(), cube_mesh.clone(), rgb,
        );
    }
}

fn egui_to_rgb(c: bevy_egui::egui::Color32) -> [f32; 3] {
    let f = |v: u8| (v as f32) / 255.0;
    [f(c.r()), f(c.g()), f(c.b())]
}

fn make_axis_material(
    materials: &mut Assets<StandardMaterial>,
    rgb: [f32; 3],
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: Color::srgb(rgb[0], rgb[1], rgb[2]),
        emissive: LinearRgba::new(rgb[0], rgb[1], rgb[2], 1.0) * EMISSIVE_BASE,
        unlit: true,
        alpha_mode: AlphaMode::Opaque,
        cull_mode: None,
        ..default()
    })
}

fn spawn_handle_parent(
    commands: &mut Commands,
    root: Entity,
    mode: GizmoMode,
    local_axis: Vec3,
    shape: GizmoShape,
    material: Handle<StandardMaterial>,
    base_rgb: [f32; 3],
    name: &str,
) -> Entity {
    commands
        .spawn((
            Name::new(name.to_string()),
            BigSpatialBundle::default(),
            GizmoHandle {
                mode,
                local_axis,
                shape,
                material,
                base_rgb,
            },
            Visibility::Hidden,
        ))
        .insert(ChildOf(root))
        .id()
}

fn spawn_arrow(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    root: Entity,
    local_axis: Vec3,
    axis_rot: Quat,
    shaft: Handle<Mesh>,
    tip: Handle<Mesh>,
    rgb: [f32; 3],
) {
    let mat = make_axis_material(materials, rgb);
    let handle = spawn_handle_parent(
        commands, root, GizmoMode::Translate, local_axis, GizmoShape::Axis,
        mat.clone(), rgb, "GizmoArrow",
    );
    let axis = commands
        .spawn((Transform::from_rotation(axis_rot), Visibility::Inherited))
        .insert(ChildOf(handle))
        .id();
    commands
        .spawn((
            Transform::from_xyz(0.0, SHAFT_LEN * 0.5, 0.0),
            Mesh3d(shaft),
            MeshMaterial3d(mat.clone()),
            Visibility::Inherited,
            NotShadowCaster,
        ))
        .insert(ChildOf(axis));
    commands
        .spawn((
            Transform::from_xyz(0.0, SHAFT_LEN, 0.0),
            Mesh3d(tip),
            MeshMaterial3d(mat),
            Visibility::Inherited,
            GizmoMeshRole::Tip,
            NotShadowCaster,
        ))
        .insert(ChildOf(axis));
}

fn spawn_ring(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    root: Entity,
    local_axis: Vec3,
    axis_rot: Quat,
    mesh: Handle<Mesh>,
    rgb: [f32; 3],
) {
    let mat = make_axis_material(materials, rgb);
    let handle = spawn_handle_parent(
        commands, root, GizmoMode::Rotate, local_axis, GizmoShape::Ring,
        mat.clone(), rgb, "GizmoRing",
    );
    commands
        .spawn((
            Transform::from_rotation(axis_rot),
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            Visibility::Inherited,
            NotShadowCaster,
        ))
        .insert(ChildOf(handle));
}

fn spawn_scale_stub(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    root: Entity,
    local_axis: Vec3,
    axis_rot: Quat,
    shaft: Handle<Mesh>,
    cube: Handle<Mesh>,
    rgb: [f32; 3],
) {
    let mat = make_axis_material(materials, rgb);
    let handle = spawn_handle_parent(
        commands, root, GizmoMode::Scale, local_axis, GizmoShape::Axis,
        mat.clone(), rgb, "GizmoScaleStub",
    );
    let axis = commands
        .spawn((Transform::from_rotation(axis_rot), Visibility::Inherited))
        .insert(ChildOf(handle))
        .id();
    commands
        .spawn((
            Transform::from_xyz(0.0, SHAFT_LEN * 0.5, 0.0),
            Mesh3d(shaft),
            MeshMaterial3d(mat.clone()),
            Visibility::Inherited,
            NotShadowCaster,
        ))
        .insert(ChildOf(axis));
    commands
        .spawn((
            Transform::from_xyz(0.0, SHAFT_LEN, 0.0),
            Mesh3d(cube),
            MeshMaterial3d(mat),
            Visibility::Inherited,
            GizmoMeshRole::Cube,
            NotShadowCaster,
        ))
        .insert(ChildOf(axis));
}

/// Per-frame: position / rotate / scale each handle to match the
/// selected vehicle, and toggle visibility by current mode.
pub fn update_transform_gizmos(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    mode: Res<GizmoMode>,
    clock: Res<SimClock>,
    mut q: Query<(&GizmoHandle, &mut Transform, &mut Visibility)>,
) {
    // Gizmos are an edit-mode affordance only: hidden while physics is
    // running so they don't drag with the vehicle frame-over-frame.
    let edit_mode = clock.paused;
    let Some(id) = selection.vehicle else {
        for (_, _, mut vis) in q.iter_mut() {
            *vis = Visibility::Hidden;
        }
        return;
    };
    let Some(state) = sim.0.vehicle(id) else {
        for (_, _, mut vis) in q.iter_mut() {
            *vis = Visibility::Hidden;
        }
        return;
    };
    if !edit_mode {
        for (_, _, mut vis) in q.iter_mut() {
            *vis = Visibility::Hidden;
        }
        return;
    }

    let pose = sim.0.vehicle_pose(id);
    let rot = Quat::from_xyzw(
        pose.rotation.x as f32,
        pose.rotation.y as f32,
        pose.rotation.z as f32,
        pose.rotation.w as f32,
    );
    let center = Vec3::new(
        pose.point.x as f32,
        pose.point.y as f32,
        pose.point.z as f32,
    );
    let size = Vec3::new(
        state.spec.chassis.size.x as f32,
        state.spec.chassis.size.y as f32,
        state.spec.chassis.size.z as f32,
    );
    let reach = size.max_element() * 0.8 + 0.5;

    for (handle, mut tr, mut vis) in q.iter_mut() {
        *vis = if handle.mode == *mode {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };

        tr.translation = center;
        tr.rotation = rot;
        tr.scale = Vec3::splat(reach);
    }
}

/// Hover-pick: brighten the emissive of whichever visible handle the
/// cursor's looking at. No-op when nothing's selected or the cursor
/// is over an egui panel.
pub fn hover_transform_gizmos(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    mode: Res<GizmoMode>,
    clock: Res<SimClock>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut contexts: EguiContexts,
    mut hovered: ResMut<HoveredGizmo>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    q: Query<(Entity, &GizmoHandle)>,
) {
    let over_ui = contexts
        .ctx_mut()
        .map(|c| c.wants_pointer_input())
        .unwrap_or(false);

    // Clear any stale hover unless we actually find a new one.
    let mut new_hover: Option<Entity> = None;

    'pick: {
        if over_ui { break 'pick; }
        // Gizmos aren't visible while playing — no picks then.
        if !clock.paused { break 'pick; }
        let Some(id) = selection.vehicle else { break 'pick };
        let Some(state) = sim.0.vehicle(id) else { break 'pick };
        let Ok(window) = windows.single() else { break 'pick };
        let Some(cursor) = window.cursor_position() else { break 'pick };
        let Ok((camera, cam_tr)) = cameras.single() else { break 'pick };
        let Ok(ray) = camera.viewport_to_world(cam_tr, cursor) else { break 'pick };
        let origin = ray.origin;
        let dir = *ray.direction;

        let pose = sim.0.vehicle_pose(id);
        let vehicle_rot = Quat::from_xyzw(
            pose.rotation.x as f32,
            pose.rotation.y as f32,
            pose.rotation.z as f32,
            pose.rotation.w as f32,
        );
        let center = Vec3::new(
            pose.point.x as f32,
            pose.point.y as f32,
            pose.point.z as f32,
        );
        let size = Vec3::new(
            state.spec.chassis.size.x as f32,
            state.spec.chassis.size.y as f32,
            state.spec.chassis.size.z as f32,
        );
        let reach = size.max_element() * 0.8 + 0.5;

        let mut best: Option<(Entity, f32)> = None;
        for (entity, handle) in q.iter() {
            if handle.mode != *mode { continue; }

            let world_axis = (vehicle_rot * handle.local_axis).normalize();
            let t_hit = match handle.shape {
                GizmoShape::Axis => {
                    // Capsule / thick line from `center` along
                    // `world_axis`, length = reach.
                    ray_axis_hit(origin, dir, center, world_axis,
                                 reach, AXIS_HIT_RADIUS * reach)
                }
                GizmoShape::Ring => {
                    // Annulus in the plane with normal `world_axis`,
                    // at radius RING_MAJOR * reach.
                    ray_ring_hit(origin, dir, center, world_axis,
                                 RING_MAJOR * reach, RING_HIT_BAND * reach)
                }
            };
            if let Some(t) = t_hit {
                if best.map_or(true, |(_, bt)| t < bt) {
                    best = Some((entity, t));
                }
            }
        }
        new_hover = best.map(|(e, _)| e);
    }

    if new_hover == hovered.0 {
        return;
    }

    // Apply the transition: restore the old handle, brighten the new.
    if let Some(prev) = hovered.0 {
        if let Ok((_, h)) = q.get(prev) {
            if let Some(m) = materials.get_mut(&h.material) {
                m.emissive = LinearRgba::new(h.base_rgb[0], h.base_rgb[1], h.base_rgb[2], 1.0)
                    * EMISSIVE_BASE;
            }
        }
    }
    if let Some(curr) = new_hover {
        if let Ok((_, h)) = q.get(curr) {
            if let Some(m) = materials.get_mut(&h.material) {
                m.emissive = LinearRgba::new(h.base_rgb[0], h.base_rgb[1], h.base_rgb[2], 1.0)
                    * EMISSIVE_HOVER;
            }
        }
    }

    hovered.0 = new_hover;
}

// ── Ray intersection helpers ────────────────────────────────────────

/// Ray vs axis-aligned capsule (finite cylinder of radius `r`, from
/// `base` along `axis` for `length` units). Returns the near `t` of
/// the ray if it enters the cylinder, ignoring hits behind the origin.
fn ray_axis_hit(
    origin: Vec3,
    dir: Vec3,
    base: Vec3,
    axis: Vec3,
    length: f32,
    radius: f32,
) -> Option<f32> {
    // Closed-form ray-vs-infinite-cylinder (axis through `base`):
    //   q(t) = (origin + t·dir - base)  projected ⟂ axis
    //   |q(t)|² = r²
    // Solve the quadratic a·t² + b·t + c = 0.
    let ad = axis.dot(dir);
    let ao = axis.dot(origin - base);
    let w = dir - axis * ad;
    let v = (origin - base) - axis * ao;
    let a = w.dot(w);
    let b = 2.0 * w.dot(v);
    let c = v.dot(v) - radius * radius;

    if a < 1e-8 {
        return None; // ray parallel to axis — unlikely to "hit" a thin handle
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let sqrt_d = disc.sqrt();
    let t0 = (-b - sqrt_d) / (2.0 * a);
    let t1 = (-b + sqrt_d) / (2.0 * a);
    // Pick nearest non-negative t.
    let t = if t0 >= 0.0 { t0 } else if t1 >= 0.0 { t1 } else { return None };
    // Ensure the hit point lies within the finite cylinder's length.
    let s = ao + ad * t;
    if s < 0.0 || s > length {
        return None;
    }
    Some(t)
}

/// Ray vs annulus (circle of `radius` lying in the plane through
/// `center` with normal `axis`, with a radial tolerance `band`).
fn ray_ring_hit(
    origin: Vec3,
    dir: Vec3,
    center: Vec3,
    axis: Vec3,
    radius: f32,
    band: f32,
) -> Option<f32> {
    let denom = dir.dot(axis);
    if denom.abs() < 1e-6 {
        return None; // ray parallel to ring plane
    }
    let t = (center - origin).dot(axis) / denom;
    if t < 0.0 {
        return None;
    }
    let hit = origin + dir * t;
    let r = (hit - center).length();
    if (r - radius).abs() <= band * 0.5 {
        Some(t)
    } else {
        None
    }
}

// ── Drag ────────────────────────────────────────────────────────────

/// LMB-press-on-hovered → start a drag, captured at press time so
/// subsequent mouse motion produces absolute deltas (no accumulated
/// floating-point drift). LMB release → end drag.
pub fn gizmo_drag_system(
    mouse: Res<ButtonInput<MouseButton>>,
    hovered: Res<HoveredGizmo>,
    selection: Res<Selection>,
    clock: Res<SimClock>,
    mut sim: ResMut<GearboxSim>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut contexts: EguiContexts,
    mut drag: ResMut<GizmoDrag>,
    handles: Query<&GizmoHandle>,
) {
    // Dragging the handles only makes sense in edit-mode (paused). In
    // play-mode the vehicle integrates under physics — trying to drive
    // a pose override every frame would fight with the rigid body.
    if !clock.paused {
        drag.active = None;
        return;
    }
    let over_ui = contexts
        .ctx_mut()
        .map(|c| c.wants_pointer_input())
        .unwrap_or(false);

    // End-of-drag on release.
    if mouse.just_released(MouseButton::Left) {
        drag.active = None;
    }

    // Need a cursor ray for everything below.
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else { return };
    let Ok((camera, cam_tr)) = cameras.single() else { return };
    let Ok(ray) = camera.viewport_to_world(cam_tr, cursor) else { return };
    let ro = ray.origin;
    let rd = *ray.direction;

    // ── Start drag on fresh press over a hovered handle ──
    if drag.active.is_none()
        && mouse.just_pressed(MouseButton::Left)
        && !over_ui
    {
        if let Some(handle_entity) = hovered.0 {
            if let Ok(h) = handles.get(handle_entity) {
                if let Some(id) = selection.vehicle {
                    let pose = sim.0.vehicle_pose(id);
                    let vehicle_rot = quat_from_pose(&pose);
                    let world_axis = (vehicle_rot * h.local_axis).normalize();
                    let center = Vec3::new(
                        pose.point.x as f32,
                        pose.point.y as f32,
                        pose.point.z as f32,
                    );

                    let (start_param, e1, e2) = match h.shape {
                        GizmoShape::Axis => {
                            let t = line_line_closest_param(ro, rd, center, world_axis);
                            (t, Vec3::ZERO, Vec3::ZERO)
                        }
                        GizmoShape::Ring => {
                            if let Some(hit) =
                                ray_plane_hit(ro, rd, center, world_axis)
                            {
                                let (b1, b2) = plane_basis(world_axis);
                                let v = hit - center;
                                let angle = v.dot(b2).atan2(v.dot(b1));
                                (angle, b1, b2)
                            } else {
                                (0.0, Vec3::ZERO, Vec3::ZERO)
                            }
                        }
                    };

                    drag.active = Some(ActiveDrag {
                        mode: h.mode,
                        shape: h.shape,
                        vehicle: id,
                        world_axis,
                        start_pose: pose,
                        start_param,
                        basis_e1: e1,
                        basis_e2: e2,
                    });
                }
            }
        }
    }

    // ── Continue drag ──
    let Some(active) = &drag.active else { return };
    if !mouse.pressed(MouseButton::Left) {
        drag.active = None;
        return;
    }

    // Scale currently isn't applied (rapier colliders are baked at
    // spawn) — keep the drag state so press+release feels consistent,
    // but skip the pose write.
    if active.mode == GizmoMode::Scale {
        return;
    }

    let start_center = Vec3::new(
        active.start_pose.point.x as f32,
        active.start_pose.point.y as f32,
        active.start_pose.point.z as f32,
    );

    let new_pose = match active.shape {
        GizmoShape::Axis => {
            // Slide the vehicle along `world_axis` by however much
            // the cursor-to-axis parameter has moved.
            let t_now = line_line_closest_param(ro, rd, start_center, active.world_axis);
            let delta = t_now - active.start_param;
            let new_center = start_center + active.world_axis * delta;
            Pose {
                point: Point::new(
                    new_center.x as f64,
                    new_center.y as f64,
                    new_center.z as f64,
                ),
                rotation: active.start_pose.rotation,
            }
        }
        GizmoShape::Ring => {
            // Rotate around `world_axis` by (current angle − press angle).
            let Some(hit) = ray_plane_hit(ro, rd, start_center, active.world_axis) else {
                return;
            };
            let v = hit - start_center;
            let angle_now = v.dot(active.basis_e2).atan2(v.dot(active.basis_e1));
            let delta = angle_now - active.start_param;
            let delta_q = Quat::from_axis_angle(active.world_axis, delta);
            let start_q = quat_from_pose(&active.start_pose);
            let new_q = (delta_q * start_q).normalize();
            Pose {
                point: active.start_pose.point,
                rotation: Quaternion::new(
                    new_q.w as f64,
                    new_q.x as f64,
                    new_q.y as f64,
                    new_q.z as f64,
                ),
            }
        }
    };

    sim.0.set_vehicle_pose(active.vehicle, new_pose);
}

fn quat_from_pose(p: &Pose) -> Quat {
    Quat::from_xyzw(
        p.rotation.x as f32,
        p.rotation.y as f32,
        p.rotation.z as f32,
        p.rotation.w as f32,
    )
    .normalize()
}

/// Parameter `t` along line-A (through `la_origin`, direction
/// `la_dir`, unit-length) that's closest to line-B (ray). Standard
/// two-line closest-point formula; collapses gracefully when the
/// lines are parallel.
fn line_line_closest_param(
    ray_origin: Vec3,
    ray_dir: Vec3,
    la_origin: Vec3,
    la_dir: Vec3,
) -> f32 {
    let w = la_origin - ray_origin;
    let b = la_dir.dot(ray_dir);
    let d = la_dir.dot(w);
    let e = ray_dir.dot(w);
    let denom = 1.0 - b * b; // a·c − b² where a=c=1 (unit dirs)
    if denom.abs() < 1e-6 {
        return 0.0;
    }
    (b * e - d) / denom
}

fn ray_plane_hit(origin: Vec3, dir: Vec3, plane_pt: Vec3, normal: Vec3) -> Option<Vec3> {
    let denom = dir.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (plane_pt - origin).dot(normal) / denom;
    if t < 0.0 {
        return None;
    }
    Some(origin + dir * t)
}

/// Arbitrary orthonormal basis for the plane whose normal is `n`.
fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    let ref_axis = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let e1 = (ref_axis - n * n.dot(ref_axis)).normalize();
    let e2 = n.cross(e1);
    (e1, e2)
}

// ── Mesh regeneration ──────────────────────────────────────────────
//
// `GizmoScale` (slider) changes world-space thickness / tip-size
// directly; it does NOT change the gizmo's overall length, which is
// driven by the selected vehicle's reach. Those two want different
// scaling behaviour, so we:
//   1. Keep the parent `Transform::scale = reach` uniform (handles all
//      *length* dimensions cleanly for any vehicle rotation).
//   2. Regenerate the four shared meshes with *unit-space* radii
//      computed as `BASE_world × slider / reach` → once the parent
//      scales by `reach` the world size resolves to `BASE × slider`,
//      independent of the vehicle.
//
// Runs only when `(reach, slider)` actually change — tracked via a
// `Local` — so mesh buffers aren't re-uploaded every frame.
pub fn regenerate_gizmo_meshes(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    gizmo_scale: Res<GizmoScale>,
    handles: Option<Res<GizmoMeshHandles>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tip_cube_q: Query<(&GizmoMeshRole, &mut Transform)>,
    mut last: Local<Option<(f32, f32)>>,
) {
    let Some(handles) = handles else { return };
    let Some(id) = selection.vehicle else { return };
    let Some(state) = sim.0.vehicle(id) else { return };

    let size = state.spec.chassis.size;
    let max_dim = (size.x.max(size.y).max(size.z)) as f32;
    let reach = max_dim * 0.8 + 0.5;
    let slider = gizmo_scale.0;

    // Skip if nothing relevant changed since last pass.
    if let Some((prev_r, prev_s)) = *last {
        if (prev_r - reach).abs() < 0.005 && (prev_s - slider).abs() < 0.0005 {
            return;
        }
    }
    *last = Some((reach, slider));

    // Unit-space sizes — multiplied back up to world by the parent
    // `scale = reach` in `update_transform_gizmos`.
    let shaft_r = BASE_SHAFT_RADIUS_W * slider / reach;
    let tip_r   = BASE_TIP_RADIUS_W   * slider / reach;
    let tip_len = BASE_TIP_LEN_W      * slider / reach;
    let cube_e  = BASE_CUBE_EDGE_W    * slider / reach;
    let ring_m  = BASE_RING_MINOR_W   * slider / reach;

    // ── Replace mesh data in-place ──
    if let Some(m) = meshes.get_mut(&handles.shaft) {
        *m = Cylinder::new(shaft_r, SHAFT_LEN).mesh().resolution(16).build();
    }
    if let Some(m) = meshes.get_mut(&handles.tip) {
        *m = Cone::new(tip_r, tip_len).mesh().resolution(16).build();
    }
    if let Some(m) = meshes.get_mut(&handles.cube) {
        *m = Cuboid::new(cube_e, cube_e, cube_e).into();
    }
    if let Some(m) = meshes.get_mut(&handles.ring) {
        *m = Torus::new(RING_MAJOR - ring_m, RING_MAJOR + ring_m)
            .mesh()
            .minor_resolution(8)
            .major_resolution(96)
            .build();
    }

    // ── Move tip/cube children to the end of the (now-variable-length) shaft ──
    for (role, mut tr) in tip_cube_q.iter_mut() {
        let y = match role {
            GizmoMeshRole::Tip  => SHAFT_LEN + tip_len * 0.5,
            GizmoMeshRole::Cube => SHAFT_LEN + cube_e  * 0.5,
        };
        tr.translation = Vec3::new(0.0, y, 0.0);
    }
}
