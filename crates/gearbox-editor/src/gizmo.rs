//! Transform-gizmo visual overlay — single-file, one shader, one
//! custom Material, one per-frame mesh rebuild.
//!
//!   - `shaders/gizmo.wgsl` is embedded via `embedded_asset!`.
//!   - `GizmoOverlayMaterial` is a zero-uniform `Material` whose
//!     vertex shader emits NDC positions straight through (no
//!     view/proj) and whose pipeline disables depth testing +
//!     enables alpha blending.
//!   - Every frame, `draw_gizmo_system` reads the editor's
//!     selection + mode-enable toggles + hovered handle, builds a
//!     `DrawList` of 2D triangles in NDC, and uploads it to the
//!     overlay entity's mesh asset.
//!
//! Pick / drag logic stays in `super::transform_gizmos`. This file
//! is the rendering layer only.

use std::f32::consts::TAU;

use bevy::asset::{embedded_asset, Asset, Handle, RenderAssetUsages};
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::world::World;
use bevy::mesh::{Indices, Mesh, MeshVertexBufferLayoutRef, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::{Material, MaterialPipeline, MaterialPlugin};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use gearbox_physics::VehicleId;
use gearbox_viz::{GearboxSim, SimClock};

use super::selection::Selection;
use super::transform_gizmos::{
    gizmo_reach, GizmoHandle, GizmoMode, GizmoModesEnabled, GizmoShape, HoveredGizmo,
    RING_MAJOR, SHAFT_LEN,
};

// ═══ Visual tunables ════════════════════════════════════════════════

/// Stroke thickness in pixels at the reference camera distance
/// (where the gizmo projects to [`REFERENCE_REACH_PX`] on screen).
/// All pixel-space constants below scale linearly with the on-screen
/// size of the gizmo — zoom out and the whole thing, including its
/// line weights, shrinks together so the arrows don't look like
/// heavy beams overlaying a tiny ring.
const STROKE_PX:         f32 = 3.0;
/// Arrow-tip base width + height in pixels (at reference size).
const ARROW_HEAD_W_PX:   f32 = 12.0;
const ARROW_HEAD_H_PX:   f32 = 18.0;
/// Scale-mode cube edge in pixels (at reference size).
const CUBE_EDGE_PX:      f32 = 12.0;
/// Projected gizmo reach in pixels that corresponds to the pixel
/// constants above. When the real projected reach is larger we scale
/// strokes up; smaller, we scale down — keeping a constant ratio of
/// "stroke thickness : gizmo size" across camera distances.
const REFERENCE_REACH_PX: f32 = 130.0;
/// Hard floor / ceiling on the stroke-scale factor so far-away
/// gizmos don't disappear and close-up ones don't get comically fat.
const STROKE_SCALE_MIN:  f32 = 0.35;
const STROKE_SCALE_MAX:  f32 = 2.5;
/// Rotation ring polygon resolution.
const RING_SEGMENTS:     u32 = 96;
/// Hover multiplier applied to each channel. High enough that most
/// channels saturate to 1.0 on hover — that's what reads as a glowy
/// "just light up the whole axis" effect without a real bloom pass.
const HOVER_BRIGHTEN:    f32 = 3.0;
/// Base gizmo alpha. 1.0 so the punchy colours come through without
/// any translucency sapping their intensity.
const FILL_ALPHA:        f32 = 1.0;

/// Gizmo-specific axis palette. Deliberately more saturated than the
/// inspector's `style::AXIS_*` glyph colours — transform-gizmo-style
/// "video-game bright" rather than data-viz subtle.
const GIZMO_AXIS_X: [f32; 3] = [0.95, 0.15, 0.25];  // vivid red
const GIZMO_AXIS_Y: [f32; 3] = [0.28, 0.85, 0.18];  // vivid green
const GIZMO_AXIS_Z: [f32; 3] = [0.15, 0.50, 1.00];  // vivid blue

/// Inner framing circle painted at the gizmo's centre. Stroked, not
/// filled — matches the outer ring's look. Radius is in pixels at
/// reference size (scaled with the rest).
const CENTER_DOT_PX:     f32 = 16.0;
/// Extra pixel gap between the inner circle's outer stroke edge and
/// the tail of each translate / scale arrow shaft, so the arrow
/// doesn't poke into the ring.
const INNER_ARROW_GAP_PX: f32 = 2.0;
/// Outer framing circle that encloses every handle. Radius is given
/// in fractions of the projected gizmo reach — slightly under the
/// scale-cube distance (0.95·reach) so it reads as an inner frame
/// rather than spilling past the extreme handle.
const OUTER_RING_FRAC:   f32 = 0.972;
/// Pixel-space circle resolution (at reference size — scaled).
const FLAT_CIRCLE_SEGS:  u32 = 72;
/// Pre-multiplied-alpha white for the two framing circles.
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 0.96];

// ═══ Plugin ═════════════════════════════════════════════════════════

pub struct GizmoOverlayPlugin;

impl Plugin for GizmoOverlayPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/gizmo.wgsl");
        app.add_plugins(MaterialPlugin::<GizmoOverlayMaterial>::default());
    }
}

// ═══ Material ═══════════════════════════════════════════════════════

#[derive(Asset, AsBindGroup, TypePath, Clone, Default)]
pub struct GizmoOverlayMaterial {}

impl Material for GizmoOverlayMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://gearbox_editor/shaders/gizmo.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox_editor/shaders/gizmo.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Pin vertex layout to just position@0 + colour@1 so it lines
        // up with `shaders/gizmo.wgsl`'s two inputs.
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];

        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_compare = CompareFunction::Always;
            ds.depth_write_enabled = false;
        }
        Ok(())
    }
}

// ═══ Overlay entity ═════════════════════════════════════════════════

#[derive(Component)]
pub struct GizmoOverlay {
    pub mesh:     Handle<Mesh>,
    pub material: Handle<GizmoOverlayMaterial>,
}

pub fn setup_gizmo_overlay(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GizmoOverlayMaterial>>,
) {
    let mesh = meshes.add(empty_mesh());
    let material = materials.add(GizmoOverlayMaterial::default());

    // The shader ignores world/view/proj — every vertex comes in
    // already in NDC — so this entity's `Transform` is purely a
    // placeholder.
    commands.spawn((
        Name::new("GizmoOverlay"),
        GizmoOverlay {
            mesh: mesh.clone(),
            material: material.clone(),
        },
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        bevy::light::NotShadowCaster,
        bevy::light::NotShadowReceiver,
        Visibility::Hidden,
        RenderLayers::layer(0),
    ));
}

fn empty_mesh() -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR,    Vec::<[f32; 4]>::new());
    m.insert_indices(Indices::U32(Vec::new()));
    m
}

// ═══ 2D shape builder ═══════════════════════════════════════════════
//
// All positions end up in NDC (`x,y ∈ [-1, 1]`, `z` is depth — the
// shader respects but we override depth testing, so any value is
// fine; 0.5 reads cleanly).

struct DrawList {
    positions: Vec<[f32; 3]>,
    colors:    Vec<[f32; 4]>,
    indices:   Vec<u32>,
    /// Cached pixel → NDC scale (`2 / viewport_size_px`). Multiply a
    /// pixel-space offset by this to land in NDC.
    ndc_per_px: Vec2,
}

impl DrawList {
    fn new(viewport_size: Vec2) -> Self {
        Self {
            positions: Vec::new(),
            colors:    Vec::new(),
            indices:   Vec::new(),
            ndc_per_px: Vec2::new(2.0 / viewport_size.x, 2.0 / viewport_size.y),
        }
    }

    fn push_vertex(&mut self, ndc: Vec2, color: [f32; 4]) -> u32 {
        let idx = self.positions.len() as u32;
        self.positions.push([ndc.x, ndc.y, 0.5]);
        self.colors.push(color);
        idx
    }

    fn push_tri(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// Thick line in screen space — pixel-width quad, two triangles.
    fn thick_line(&mut self, a: Vec2, b: Vec2, width_px: f32, color: [f32; 4]) {
        let dir = (b - a).normalize_or_zero();
        if dir == Vec2::ZERO { return; }
        let n_ndc = Vec2::new(-dir.y, dir.x) * (width_px * 0.5) * self.ndc_per_px;
        let v0 = self.push_vertex(a + n_ndc, color);
        let v1 = self.push_vertex(a - n_ndc, color);
        let v2 = self.push_vertex(b + n_ndc, color);
        let v3 = self.push_vertex(b - n_ndc, color);
        self.push_tri(v0, v1, v2);
        self.push_tri(v1, v3, v2);
    }

    /// Filled triangle — used for arrowheads.
    fn triangle(&mut self, a: Vec2, b: Vec2, c: Vec2, color: [f32; 4]) {
        let i0 = self.push_vertex(a, color);
        let i1 = self.push_vertex(b, color);
        let i2 = self.push_vertex(c, color);
        self.push_tri(i0, i1, i2);
    }

    /// Filled convex polygon via a triangle fan.
    fn convex_polygon(&mut self, verts: &[Vec2], color: [f32; 4]) {
        if verts.len() < 3 { return; }
        let base = self.push_vertex(verts[0], color);
        let mut prev = self.push_vertex(verts[1], color);
        for v in &verts[2..] {
            let curr = self.push_vertex(*v, color);
            self.push_tri(base, prev, curr);
            prev = curr;
        }
    }

    /// Filled disk in screen space — always faces the camera.
    /// Triangle-fan around `centre_ndc` with radius in pixels.
    fn flat_disk(&mut self, centre_ndc: Vec2, radius_px: f32, color: [f32; 4], segments: u32) {
        let seg = segments.max(3);
        let step = TAU / seg as f32;
        let r_ndc = radius_px * self.ndc_per_px;
        let centre_idx = self.push_vertex(centre_ndc, color);
        let mut prev = 0u32;
        for i in 0..=seg {
            let t = i as f32 * step;
            let (s, c) = t.sin_cos();
            let p = centre_ndc + Vec2::new(c, s) * r_ndc;
            let curr = self.push_vertex(p, color);
            if i > 0 {
                self.push_tri(centre_idx, prev, curr);
            }
            prev = curr;
        }
    }

    /// Stroked (outline-only) circle in screen space — always faces
    /// the camera. Built as `segments` thick-line segments.
    fn flat_ring_stroke(
        &mut self,
        centre_ndc: Vec2,
        radius_px: f32,
        width_px: f32,
        color: [f32; 4],
        segments: u32,
    ) {
        let seg = segments.max(8);
        let step = TAU / seg as f32;
        let r_ndc = radius_px * self.ndc_per_px;
        let mut prev = centre_ndc + Vec2::new(1.0, 0.0) * r_ndc;
        for i in 1..=seg {
            let t = i as f32 * step;
            let (s, c) = t.sin_cos();
            let curr = centre_ndc + Vec2::new(c, s) * r_ndc;
            self.thick_line(prev, curr, width_px, color);
            prev = curr;
        }
    }

    /// Ring: traces a circle in world space around `centre_world`
    /// with plane normal `axis_world`, projects each sampled point to
    /// NDC, joins them with thick-line segments.
    fn projected_ring(
        &mut self,
        proj: &impl Fn(Vec3) -> Option<Vec2>,
        centre_world: Vec3,
        axis_world: Vec3,
        radius_world: f32,
        width_px: f32,
        color: [f32; 4],
        segments: u32,
    ) {
        let (e1, e2) = plane_basis(axis_world);
        let seg = segments.max(8);
        let step = TAU / seg as f32;
        let mut pts: Vec<Vec2> = Vec::with_capacity(seg as usize);
        for i in 0..seg {
            let t = i as f32 * step;
            let (s, c) = t.sin_cos();
            let w = centre_world + e1 * (c * radius_world) + e2 * (s * radius_world);
            if let Some(p) = proj(w) { pts.push(p); }
        }
        if pts.len() < 2 { return; }
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            self.thick_line(a, b, width_px, color);
        }
    }

    fn write_into(self, mesh: &mut Mesh) {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            VertexAttributeValues::Float32x3(self.positions),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            VertexAttributeValues::Float32x4(self.colors),
        );
        mesh.insert_indices(Indices::U32(self.indices));
    }
}

// ═══ Draw system ════════════════════════════════════════════════════

pub fn draw_gizmo_system(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    modes_enabled: Res<GizmoModesEnabled>,
    hovered: Res<HoveredGizmo>,
    clock: Res<SimClock>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection)>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    handles: Query<(Entity, &GizmoHandle)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut overlay: Query<(&GizmoOverlay, &mut Visibility)>,
) {
    let Ok((overlay_data, mut vis)) = overlay.single_mut() else { return };
    let Some(mesh) = meshes.get_mut(&overlay_data.mesh) else { return };

    // Show the gizmo only in edit mode, on a valid selection, with at
    // least one mode enabled.
    let do_draw = clock.paused && selection.vehicle.is_some() && modes_enabled.any();
    if !do_draw {
        *vis = Visibility::Hidden;
        *mesh = empty_mesh();
        return;
    }

    let Some(id) = selection.vehicle else { return };
    let Some(state) = sim.0.vehicle(id) else {
        *vis = Visibility::Hidden;
        return;
    };

    let Ok(window) = windows.single() else { return };
    let Ok((camera, cam_global, projection)) = cameras.single() else { return };
    let win_w = window.resolution.physical_width()  as f32;
    let win_h = window.resolution.physical_height() as f32;
    if win_w <= 0.0 || win_h <= 0.0 { return; }

    let Some((center, vehicle_rot)) = vehicle_xform(&sim.0, id) else { return };
    // Vehicle-relative reach — camera zoom doesn't affect it, so the
    // gizmo stays the same world size regardless of how close/far
    // the camera sits.
    let vehicle_size = Vec3::new(
        state.spec.chassis.size.x as f32,
        state.spec.chassis.size.y as f32,
        state.spec.chassis.size.z as f32,
    );
    let reach = gizmo_reach(vehicle_size);

    // Gizmo's projected pixel size — drives BOTH the stroke-scale
    // multiplier (so far-away gizmos have thinner lines) AND the
    // outer framing circle radius (so it hugs the handles at any
    // zoom).
    let projected_reach_px = projected_reach_pixels(
        cam_global, projection, win_h, center, reach,
    );
    let stroke_scale = (projected_reach_px / REFERENCE_REACH_PX)
        .clamp(STROKE_SCALE_MIN, STROKE_SCALE_MAX);
    let stroke_px   = STROKE_PX        * stroke_scale;
    let head_w_px   = ARROW_HEAD_W_PX  * stroke_scale;
    let head_h_px   = ARROW_HEAD_H_PX  * stroke_scale;
    let cube_edge   = CUBE_EDGE_PX     * stroke_scale;
    let dot_radius  = CENTER_DOT_PX    * stroke_scale;

    // Distance — as a fraction of `reach` — from the gizmo origin to
    // the outer stroke edge of the inner circle, plus a small gap.
    // Arrow shafts start at this offset along their axis so they
    // don't poke into the inner ring.
    let inner_tail_frac = ((dot_radius + stroke_px * 0.5 + INNER_ARROW_GAP_PX)
        / projected_reach_px.max(1.0))
        .clamp(0.0, 0.95);

    // World → NDC projector.
    let proj = |world: Vec3| -> Option<Vec2> {
        camera.world_to_viewport(cam_global, world)
            .ok()
            .map(|vp| Vec2::new(
                (vp.x / win_w) * 2.0 - 1.0,
                1.0 - (vp.y / win_h) * 2.0,
            ))
    };

    let mut dl = DrawList::new(Vec2::new(win_w, win_h));

    // Project the gizmo centre once — both the outer framing ring
    // and the centre dot live at that screen position.
    let centre_ndc = proj(center);

    // ── Outer framing circle ──
    // Painted FIRST so every axis handle renders on top of it. White
    // stroke, slightly larger than the farthest handle (scale cube
    // at 0.95·reach). Always a flat 2D circle — no 3D tilt — so it
    // looks the same from any camera angle.
    if let Some(c) = centre_ndc {
        dl.flat_ring_stroke(
            c,
            projected_reach_px * OUTER_RING_FRAC,
            stroke_px,
            WHITE,
            FLAT_CIRCLE_SEGS,
        );
    }

    // All three modes visible simultaneously (no mode filter — the
    // user clicks whichever handle they want).
    for (entity, handle) in &handles {
        if !modes_enabled.has(handle.mode) { continue; }

        let world_axis = (vehicle_rot * handle.local_axis).normalize();
        let base_rgb = axis_rgb(handle.local_axis);
        let is_hovered = hovered.0 == Some(entity);
        let color = colour_for(base_rgb, is_hovered);

        match (handle.mode, handle.shape) {
            (GizmoMode::Translate, GizmoShape::Axis) => {
                build_arrow(
                    &mut dl, &proj, center, world_axis, reach, color,
                    stroke_px, head_w_px, head_h_px, inner_tail_frac,
                );
            }
            (GizmoMode::Rotate, GizmoShape::Ring) => {
                dl.projected_ring(
                    &proj,
                    center,
                    world_axis,
                    RING_MAJOR * reach,
                    stroke_px,
                    color,
                    RING_SEGMENTS,
                );
            }
            (GizmoMode::Scale, GizmoShape::Axis) => {
                build_scale_stub(
                    &mut dl, &proj, center, world_axis, reach, color,
                    cube_edge,
                );
            }
            _ => {}
        }
    }

    // ── Inner framing ring ──
    // Same stroked-circle look as the outer frame, just smaller.
    // Painted LAST so it sits on top of any stray pixel from the
    // axis shapes near the origin.
    if let Some(c) = centre_ndc {
        dl.flat_ring_stroke(c, dot_radius, stroke_px, WHITE, FLAT_CIRCLE_SEGS);
    }

    *mesh = empty_mesh();
    dl.write_into(mesh);
    *vis = Visibility::Visible;
}

// ═══ Per-handle shape recipes ═══════════════════════════════════════

fn build_arrow(
    dl: &mut DrawList,
    proj: &impl Fn(Vec3) -> Option<Vec2>,
    center: Vec3,
    world_axis: Vec3,
    reach: f32,
    color: [f32; 4],
    shaft_px: f32,
    head_w_px: f32,
    head_h_px: f32,
    // Axis-distance (as a fraction of `reach`) to offset the tail
    // from the origin, so the shaft starts outside the inner framing
    // ring rather than poking into it.
    tail_offset_frac: f32,
) {
    let tail_w = center + world_axis * (tail_offset_frac * reach);
    let shaft_end_w = center + world_axis * (SHAFT_LEN * reach);
    let (Some(tail), Some(shaft_end)) = (proj(tail_w), proj(shaft_end_w)) else { return };

    dl.thick_line(tail, shaft_end, shaft_px, color);

    // Arrow head — filled triangle in screen space, oriented along
    // the projected direction. NDC axes are anisotropic, so turn the
    // NDC-space direction back into isotropic pixel-space for the
    // head offsets, then convert back.
    let dir_ndc = (shaft_end - tail).normalize_or_zero();
    if dir_ndc == Vec2::ZERO { return; }
    let ndc_per_px = dl.ndc_per_px;
    let dir_px = Vec2::new(dir_ndc.x / ndc_per_px.x, dir_ndc.y / ndc_per_px.y)
        .normalize_or_zero();
    if dir_px == Vec2::ZERO { return; }
    let n_px = Vec2::new(-dir_px.y, dir_px.x);

    let tip   = shaft_end + dir_px * head_h_px * ndc_per_px;
    let left  = shaft_end + n_px   * (head_w_px * 0.5) * ndc_per_px;
    let right = shaft_end - n_px   * (head_w_px * 0.5) * ndc_per_px;
    dl.triangle(left, right, tip, color);
}

fn build_scale_stub(
    dl: &mut DrawList,
    proj: &impl Fn(Vec3) -> Option<Vec2>,
    center: Vec3,
    world_axis: Vec3,
    reach: f32,
    color: [f32; 4],
    cube_edge_px: f32,
) {
    // Position the cube past the arrow tip so it reads as a distinct
    // grab target rather than merging with the arrowhead.
    const SCALE_POS_FRAC: f32 = 0.95;
    let cube_centre_w = center + world_axis * (SCALE_POS_FRAC * reach);
    let Some(cube_centre) = proj(cube_centre_w) else { return };

    let ndc_per_px = dl.ndc_per_px;
    let half = cube_edge_px * 0.5;
    let corners = [
        cube_centre + Vec2::new(-half,  half) * ndc_per_px,
        cube_centre + Vec2::new( half,  half) * ndc_per_px,
        cube_centre + Vec2::new( half, -half) * ndc_per_px,
        cube_centre + Vec2::new(-half, -half) * ndc_per_px,
    ];
    dl.convex_polygon(&corners, color);
}

// ═══ Math + colour helpers ══════════════════════════════════════════

/// How many pixels of screen the gizmo's world-space `reach` spans
/// at the camera's current distance. For a perspective camera at
/// distance `D`, vertical FOV `F`, viewport height `H px`: one world
/// unit at that depth projects to `H / (2·D·tan(F/2))` pixels.
fn projected_reach_pixels(
    cam_global: &GlobalTransform,
    projection: &Projection,
    window_height_px: f32,
    vehicle_center: Vec3,
    reach: f32,
) -> f32 {
    let distance = (vehicle_center - cam_global.translation()).length().max(0.1);
    let fov_y = match projection {
        Projection::Perspective(p) => p.fov,
        _ => std::f32::consts::FRAC_PI_4,
    };
    let pixels_per_world = window_height_px.max(1.0) / (2.0 * distance * (fov_y * 0.5).tan());
    reach * pixels_per_world
}

fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    let ref_axis = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let e1 = (ref_axis - n * n.dot(ref_axis)).normalize();
    let e2 = n.cross(e1);
    (e1, e2)
}

fn vehicle_xform(sim: &gearbox_physics::Sim, id: VehicleId) -> Option<(Vec3, Quat)> {
    let pose = sim.vehicle_pose(id);
    Some((
        Vec3::new(
            pose.point.x as f32,
            pose.point.y as f32,
            pose.point.z as f32,
        ),
        Quat::from_xyzw(
            pose.rotation.x as f32,
            pose.rotation.y as f32,
            pose.rotation.z as f32,
            pose.rotation.w as f32,
        ),
    ))
}

fn axis_rgb(local_axis: Vec3) -> [f32; 3] {
    if local_axis.x.abs() > 0.5 {
        GIZMO_AXIS_X
    } else if local_axis.y.abs() > 0.5 {
        GIZMO_AXIS_Y
    } else {
        GIZMO_AXIS_Z
    }
}

fn colour_for(rgb: [f32; 3], hover: bool) -> [f32; 4] {
    let k = if hover { HOVER_BRIGHTEN } else { 1.0 };
    [
        (rgb[0] * k).min(1.0),
        (rgb[1] * k).min(1.0),
        (rgb[2] * k).min(1.0),
        FILL_ALPHA,
    ]
}

#[allow(dead_code)]
fn _keep_world_import(_w: &World) {}
