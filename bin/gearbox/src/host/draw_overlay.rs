//! Debug draws the host paints over the viewport: in-world charts (panels)
//! and the interactions clients ask for — a Next button, picking a point
//! or a pose on the ground, and 6-DoF handles — whose answers go out as
//! `draw` scene events.

use std::collections::HashMap;

use bevy::prelude::*;
use gearbox_api::wire::draw::unpack_rgba;
use gearbox_api::wire::draw_geom::ramp;
use gearbox_api::{DrawEntity, DrawFrame, DrawStoreRes, GearboxBus, SceneEvent, colormap, draw_kind, event_kind, interact_kind, panel_kind};
use mara::ui::mara_core;
use mara::ui::modules::bevy as mara_bevy;
use transform_gizmo::math::Transform as GizmoTransform;
use transform_gizmo::{Gizmo as GizmoCore, GizmoConfig, GizmoInteraction, GizmoMode, GizmoOrientation, GizmoVisuals, Rect as GizmoRect, mint};

use crate::draw::{DrawPanel, DrawPanels, PanelChart, enu_to_site, pose_of, site_transform};

const PLATE: egui::Color32 = egui::Color32::from_rgba_premultiplied(8, 10, 14, 196);
const INK: egui::Color32 = egui::Color32::from_rgb(232, 236, 240);
const DIM: egui::Color32 = egui::Color32::from_rgb(150, 158, 168);

fn c32(c: [u8; 4], opacity: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (c[3] as f32 * opacity) as u8)
}

fn faded(c: egui::Color32, opacity: f32) -> egui::Color32 {
    c.gamma_multiply(opacity)
}

/// The panels, painted where their anchors are; legends stack in the
/// bottom-right corner.
pub fn paint_draw_panels(egui: &egui::Context, viewport: mara_core::vocab::Rect, world: &World) {
    let Some(panels) = world.get_resource::<DrawPanels>() else {
        return;
    };
    if panels.0.is_empty() {
        return;
    }
    let rect: egui::Rect = viewport.into();
    let painter = egui.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("gearbox_draw_panels"))).with_clip_rect(rect);
    let mut legend_y = rect.max.y - 18.0;
    for panel in &panels.0 {
        match panel.at {
            None => {
                let size = egui::vec2(230.0, 54.0);
                let min = egui::pos2(rect.max.x - size.x - 18.0, legend_y - size.y);
                legend(&painter, egui::Rect::from_min_size(min, size), &panel.chart);
                legend_y -= size.y + 10.0;
            }
            Some(at) => {
                let anchor = rect.min + egui::vec2(at[0] * rect.width(), at[1] * rect.height());
                chart(&painter, anchor, panel);
            }
        }
    }
}

/// A plate with an accent bar on its left and a title, sized `size`,
/// standing above `anchor` on a short leader.
fn plate(painter: &egui::Painter, anchor: egui::Pos2, size: egui::Vec2, accent: egui::Color32, title: &str, opacity: f32, leader: bool) -> egui::Rect {
    let lift = if leader { 26.0 } else { 0.0 };
    let min = anchor + egui::vec2(-0.5 * size.x, -size.y - lift);
    let r = egui::Rect::from_min_size(min, size);
    if leader {
        painter.line_segment([anchor, egui::pos2(anchor.x, r.max.y)], egui::Stroke::new(1.2, faded(accent, opacity)));
        painter.circle_filled(anchor, 2.6, faded(accent, opacity));
    }
    painter.rect_filled(r.expand(1.5), 7.0, faded(accent.gamma_multiply(0.25), opacity));
    painter.rect_filled(r, 6.0, faded(PLATE, opacity));
    let bar = egui::Rect::from_min_max(r.min + egui::vec2(0.0, 6.0), egui::pos2(r.min.x + 2.5, r.max.y - 6.0));
    painter.rect_filled(bar, 1.0, faded(accent, opacity));
    if !title.is_empty() {
        painter.text(r.min + egui::vec2(10.0, 6.0), egui::Align2::LEFT_TOP, title.to_uppercase(), egui::FontId::proportional(10.5), faded(DIM, opacity));
    }
    r
}

fn number(v: f32) -> String {
    if v.abs() >= 100.0 { format!("{v:.0}") } else if v.abs() >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") }
}

fn chart(painter: &egui::Painter, anchor: egui::Pos2, panel: &DrawPanel) {
    let c = &panel.chart;
    let o = panel.opacity;
    let accent = c32(c.color, 1.0);
    match c.kind {
        panel_kind::SPARKLINE => {
            let r = plate(painter, anchor, egui::vec2(190.0, 70.0), accent, &c.title, o, true);
            let last = c.values.last().copied().unwrap_or(0.0);
            painter.text(r.right_top() + egui::vec2(-10.0, 4.0), egui::Align2::RIGHT_TOP, format!("{} {}", number(last), c.unit), egui::FontId::monospace(14.0), faded(INK, o));
            let area = egui::Rect::from_min_max(r.min + egui::vec2(10.0, 26.0), r.max - egui::vec2(10.0, 8.0));
            if c.values.len() >= 2 {
                let (lo, hi) = c.values.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
                let span = (hi - lo).max(1e-6);
                let n = c.values.len() - 1;
                let pts: Vec<egui::Pos2> = c
                    .values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| egui::pos2(area.min.x + area.width() * i as f32 / n as f32, area.max.y - area.height() * (v - lo) / span))
                    .collect();
                for w in pts.windows(2) {
                    let quad = vec![w[0], w[1], egui::pos2(w[1].x, area.max.y), egui::pos2(w[0].x, area.max.y)];
                    painter.add(egui::Shape::convex_polygon(quad, faded(accent.gamma_multiply(0.14), o), egui::Stroke::NONE));
                }
                painter.add(egui::Shape::line(pts.clone(), egui::Stroke::new(4.0, faded(accent.gamma_multiply(0.3), o))));
                painter.add(egui::Shape::line(pts.clone(), egui::Stroke::new(1.6, faded(accent, o))));
                painter.circle_filled(*pts.last().unwrap(), 3.2, faded(egui::Color32::WHITE, o));
                painter.text(area.left_top(), egui::Align2::LEFT_TOP, number(hi), egui::FontId::monospace(9.0), faded(DIM, o));
                painter.text(area.left_bottom(), egui::Align2::LEFT_BOTTOM, number(lo), egui::FontId::monospace(9.0), faded(DIM, o));
            }
        }
        panel_kind::BARS => {
            let n = c.values.len().max(1);
            let w = (34.0 * n as f32 + 20.0).clamp(120.0, 360.0);
            let r = plate(painter, anchor, egui::vec2(w, 96.0), accent, &c.title, o, true);
            let area = egui::Rect::from_min_max(r.min + egui::vec2(12.0, 24.0), r.max - egui::vec2(10.0, 18.0));
            let top = c.values.iter().copied().fold(0.0f32, f32::max).max(1e-6);
            let hi = if c.max_given && c.max > 0.0 { c.max.max(top) } else { top * 1.15 };
            let slot = area.width() / n as f32;
            for (i, v) in c.values.iter().enumerate() {
                let col = c.colors.get(i).map(|k| c32(*k, 1.0)).unwrap_or_else(|| c32(unpack_rgba(gearbox_api::draw_color::OKABE[i % 8]), 1.0));
                let h = area.height() * (v / hi).clamp(0.0, 1.0);
                let bar = egui::Rect::from_min_max(egui::pos2(area.min.x + slot * (i as f32 + 0.18), area.max.y - h), egui::pos2(area.min.x + slot * (i as f32 + 0.82), area.max.y));
                painter.rect_filled(bar.expand(1.5), 3.0, faded(col.gamma_multiply(0.3), o));
                painter.rect_filled(bar, 2.0, faded(col, o));
                painter.text(egui::pos2(bar.center().x, bar.min.y - 2.0), egui::Align2::CENTER_BOTTOM, number(*v), egui::FontId::monospace(9.0), faded(INK, o));
                if let Some(l) = c.labels.get(i) {
                    painter.text(egui::pos2(bar.center().x, area.max.y + 2.0), egui::Align2::CENTER_TOP, l, egui::FontId::proportional(9.5), faded(DIM, o));
                }
            }
        }
        panel_kind::GAUGE => {
            let r = plate(painter, anchor, egui::vec2(120.0, 104.0), accent, &c.title, o, true);
            let centre = r.center() + egui::vec2(0.0, 12.0);
            let radius = 34.0;
            let v = c.values.first().copied().unwrap_or(0.0);
            let share = ((v - c.min) / (c.max - c.min).max(1e-6)).clamp(0.0, 1.0);
            let arc = |from: f32, to: f32| -> Vec<egui::Pos2> {
                (0..=40).map(|k| {
                    let a = from + (to - from) * k as f32 / 40.0;
                    centre + radius * egui::vec2(a.cos(), a.sin())
                }).collect()
            };
            let start = 0.75 * std::f32::consts::PI;
            let sweep = 1.5 * std::f32::consts::PI;
            let level = if share < 0.2 { c32(unpack_rgba(gearbox_api::draw_color::RED), 1.0) } else { accent };
            painter.add(egui::Shape::line(arc(start, start + sweep), egui::Stroke::new(7.0, faded(egui::Color32::from_white_alpha(28), o))));
            if share > 0.0 {
                painter.add(egui::Shape::line(arc(start, start + sweep * share), egui::Stroke::new(11.0, faded(level.gamma_multiply(0.25), o))));
                painter.add(egui::Shape::line(arc(start, start + sweep * share), egui::Stroke::new(6.0, faded(level, o))));
            }
            painter.text(centre, egui::Align2::CENTER_CENTER, number(v), egui::FontId::monospace(17.0), faded(INK, o));
            painter.text(centre + egui::vec2(0.0, 17.0), egui::Align2::CENTER_CENTER, &c.unit, egui::FontId::proportional(10.0), faded(DIM, o));
        }
        panel_kind::CHIP => {
            let text = if c.text.is_empty() { c.title.clone() } else { c.text.clone() };
            let galley = painter.layout_no_wrap(text.clone(), egui::FontId::proportional(13.0), INK);
            let size = galley.size() + egui::vec2(34.0, 12.0);
            let lift = if c.badge { 0.0 } else { 18.0 };
            let r = egui::Rect::from_center_size(anchor - egui::vec2(0.0, 0.5 * size.y + lift), size);
            if !c.badge {
                painter.line_segment([anchor, egui::pos2(anchor.x, r.max.y)], egui::Stroke::new(1.2, faded(accent, o)));
            }
            painter.rect_filled(r.expand(2.5), 0.5 * size.y + 2.5, faded(accent.gamma_multiply(0.3), o));
            painter.rect_filled(r, 0.5 * size.y, faded(PLATE, o));
            painter.rect_stroke(r, 0.5 * size.y, egui::Stroke::new(1.2, faded(accent, o)), egui::StrokeKind::Inside);
            let dot = egui::pos2(r.min.x + 12.0, r.center().y);
            painter.circle_filled(dot, 5.5, faded(accent.gamma_multiply(0.35), o));
            painter.circle_filled(dot, 3.2, faded(accent, o));
            painter.galley(egui::pos2(r.min.x + 22.0, r.center().y - 0.5 * galley.size().y), galley, faded(INK, o));
        }
        panel_kind::TABLE => {
            let rows: Vec<(String, String)> = if c.rows.is_empty() {
                c.labels.iter().zip(&c.values).map(|(l, v)| (l.clone(), format!("{} {}", number(*v), c.unit))).collect()
            } else {
                c.rows.clone()
            };
            let h = 26.0 + 16.0 * rows.len() as f32;
            let r = plate(painter, anchor, egui::vec2(180.0, h), accent, &c.title, o, true);
            for (i, (k, v)) in rows.iter().enumerate() {
                let y = r.min.y + 24.0 + 16.0 * i as f32;
                painter.text(egui::pos2(r.min.x + 10.0, y), egui::Align2::LEFT_TOP, k, egui::FontId::proportional(11.0), faded(DIM, o));
                painter.text(egui::pos2(r.max.x - 10.0, y), egui::Align2::RIGHT_TOP, v, egui::FontId::monospace(11.0), faded(INK, o));
            }
        }
        _ => {}
    }
}

/// A colormap bar from vmin to vmax with its title and ticks.
fn legend(painter: &egui::Painter, r: egui::Rect, c: &PanelChart) {
    painter.rect_filled(r, 6.0, PLATE);
    painter.text(r.min + egui::vec2(10.0, 6.0), egui::Align2::LEFT_TOP, c.title.to_uppercase(), egui::FontId::proportional(10.5), DIM);
    let bar = egui::Rect::from_min_max(r.min + egui::vec2(10.0, 22.0), egui::pos2(r.max.x - 10.0, r.min.y + 34.0));
    let (map, lo, hi) = (if c.map.0 == colormap::NONE { colormap::VIRIDIS } else { c.map.0 }, c.map.1, c.map.2);
    let steps = 48;
    for k in 0..steps {
        let t0 = k as f32 / steps as f32;
        let t1 = (k + 1) as f32 / steps as f32;
        let col = if colormap::is_palette(map) {
            gearbox_api::wire::draw_geom::palette(map, lo + (hi - lo) * t0)
        } else {
            ramp(map, if lo > hi { 1.0 - t0 } else { t0 })
        };
        let seg = egui::Rect::from_min_max(egui::pos2(bar.min.x + bar.width() * t0, bar.min.y), egui::pos2(bar.min.x + bar.width() * t1 + 0.5, bar.max.y));
        painter.rect_filled(seg, 0.0, c32(col, 1.0));
    }
    painter.rect_stroke(bar, 2.0, egui::Stroke::new(1.0, egui::Color32::from_white_alpha(60)), egui::StrokeKind::Outside);
    for (t, v) in [(0.0, lo), (0.5, 0.5 * (lo + hi)), (1.0, hi)] {
        let x = bar.min.x + bar.width() * t;
        painter.line_segment([egui::pos2(x, bar.max.y), egui::pos2(x, bar.max.y + 3.0)], egui::Stroke::new(1.0, DIM));
        let align = if t == 0.0 { egui::Align2::LEFT_TOP } else if t == 1.0 { egui::Align2::RIGHT_TOP } else { egui::Align2::CENTER_TOP };
        painter.text(egui::pos2(x, bar.max.y + 4.0), align, format!("{}{}", number(v), if t == 1.0 { format!(" {}", c.unit) } else { String::new() }), egui::FontId::monospace(9.5), INK);
    }
}

/// One live interaction request.
#[derive(Clone)]
struct Request {
    ns: String,
    id: String,
    kind: u32,
    frame: DrawFrame,
    pose: Transform,
    scale: f32,
    props: gearbox_api::Props,
}

/// Interactions in progress: the pose being picked, and a gizmo per handle.
#[derive(Default)]
pub struct DrawInteract {
    viewport: Option<egui::Rect>,
    requests: Vec<Request>,
    /// Where a pose pick was pressed: screen and entity-frame points.
    pressed: Option<(egui::Pos2, Vec3)>,
    hover: Option<egui::Pos2>,
    gizmos: HashMap<(String, String), (GizmoCore, bool)>,
    menu: Option<(String, String, egui::Pos2)>,
}

fn rows(m: Mat4) -> mint::RowMatrix4<f64> {
    let r = |i: usize| {
        let v = m.row(i);
        mint::Vector4 { x: v.x as f64, y: v.y as f64, z: v.z as f64, w: v.w as f64 }
    };
    mint::RowMatrix4 { x: r(0), y: r(1), z: r(2), w: r(3) }
}

/// The camera and the home site's world transform.
fn view(world: &mut World) -> Option<(GlobalTransform, f32, f32, GlobalTransform, crate::globe::SiteEntry)> {
    let mut q = world.query_filtered::<(&GlobalTransform, &Projection), With<mara_bevy::ChaseCamera>>();
    let (gt, projection) = q.iter(world).next().map(|(g, p)| (*g, p.clone()))?;
    let (fov, near) = match projection {
        Projection::Perspective(p) => (p.fov, p.near),
        _ => (0.8, 0.1),
    };
    let home = world.get_resource::<crate::globe::Sites>()?.home().clone();
    let site = *world.get::<GlobalTransform>(home.entity)?;
    Some((gt, fov, near, site, home))
}

/// Entity-frame coordinates of the frame into the home site's.
fn frame_to_site(frame: &DrawFrame, home: &crate::globe::SiteEntry) -> Transform {
    site_transform(frame, Transform::IDENTITY, home).unwrap_or_else(enu_to_site)
}

/// Where the ray through `cursor` meets the ground, in the home site's frame.
fn ground_hit(cursor: egui::Pos2, viewport: egui::Rect, eye: &GlobalTransform, fov: f32, site: &GlobalTransform) -> Option<Vec3> {
    let ndc = egui::vec2((cursor.x - viewport.min.x) / viewport.width() * 2.0 - 1.0, 1.0 - (cursor.y - viewport.min.y) / viewport.height() * 2.0);
    let aspect = viewport.width() / viewport.height().max(1.0);
    let t = (0.5 * fov).tan();
    let dir_view = Vec3::new(ndc.x * t * aspect, ndc.y * t, -1.0).normalize();
    let (_, rot, origin) = eye.to_scale_rotation_translation();
    let inv = site.affine().inverse();
    let o = inv.transform_point3(origin);
    let d = inv.transform_vector3(rot * dir_view).normalize();
    let above = |t: f32| {
        let p = o + d * t;
        p.y - crate::globe::ground_height_at_physics(p.x as f64, p.z as f64) as f32
    };
    let (mut lo, mut t) = (0.0f32, 0.3f32);
    if above(0.0) < 0.0 {
        return None;
    }
    while t < 4000.0 {
        if above(t) <= 0.0 {
            let mut hi = t;
            for _ in 0..24 {
                let mid = 0.5 * (lo + hi);
                if above(mid) > 0.0 { lo = mid } else { hi = mid }
            }
            return Some(o + d * hi);
        }
        lo = t;
        t += (0.02 * t).max(0.25);
    }
    None
}

fn answer(world: &mut World, r: &Request, what: &str, at: Vec3, extra: &[(&str, String)]) {
    let mut ev = SceneEvent::new(event_kind::DRAW, &r.id).at(at.x, at.y, at.z).with_prop("ns", &r.ns).with_prop("what", what);
    for (k, v) in extra {
        ev = ev.with_prop(k, v);
    }
    if let Some(mut bus) = world.get_resource_mut::<GearboxBus>() {
        bus.publish_event(ev);
    }
}

fn remove(world: &World, r: &Request) {
    if let Some(store) = world.get_resource::<DrawStoreRes>() {
        let _ = store.lock().apply(DrawEntity::delete(&r.ns, &r.id), 0.0);
    }
}

impl DrawInteract {
    /// Feed this frame's pointer to the live interactions before the
    /// viewport ticks, holding the camera still while one has the pointer.
    pub fn interact(&mut self, egui: &egui::Context, world: &mut World) {
        self.requests = world
            .get_resource::<DrawStoreRes>()
            .map(|s| {
                s.lock()
                    .iter()
                    .filter(|(_, item)| item.entity.kind == draw_kind::INTERACT)
                    .map(|((ns, id), item)| Request {
                        ns: ns.clone(),
                        id: id.clone(),
                        kind: item.entity.sub,
                        frame: item.entity.frame(),
                        pose: pose_of(&item.entity),
                        scale: item.entity.sx.abs().max(0.05),
                        props: item.entity.props(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let Some(viewport) = self.viewport else {
            return;
        };
        let Some((eye, fov, near, site, home)) = view(world) else {
            return;
        };
        let (pos, pressed, released, down, secondary) = egui.input(|i| {
            (i.pointer.latest_pos(), i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.primary_down(), i.pointer.secondary_pressed())
        });
        let over = pos.is_some_and(|p| viewport.contains(p)) && !egui.is_pointer_over_egui();
        let mut grab = false;
        self.hover = None;
        if let Some(r) = self.requests.iter().find(|r| matches!(r.kind, interact_kind::PICK_POINT | interact_kind::PICK_POSE)).cloned() {
            let to_entity = frame_to_site(&r.frame, &home).compute_affine().inverse();
            if egui.input(|i| i.key_pressed(egui::Key::Escape)) {
                answer(world, &r, "cancel", Vec3::ZERO, &[]);
                remove(world, &r);
                self.pressed = None;
                return;
            }
            if over {
                grab = true;
                self.hover = pos;
                let hit = pos.and_then(|p| ground_hit(p, viewport, &eye, fov, &site)).map(|h| to_entity.transform_point3(h));
                match r.kind {
                    interact_kind::PICK_POINT if pressed => {
                        if let Some(h) = hit {
                            answer(world, &r, "point", h, &[]);
                            remove(world, &r);
                        }
                    }
                    interact_kind::PICK_POSE => {
                        if pressed && let (Some(p), Some(h)) = (pos, hit) {
                            self.pressed = Some((p, h));
                        }
                        if released && let (Some((_, from)), Some(to)) = (self.pressed.take(), hit) {
                            let yaw = (to.y - from.y).atan2(to.x - from.x);
                            answer(world, &r, "pose", from, &[("yaw", yaw.to_string())]);
                            remove(world, &r);
                        }
                    }
                    _ => {}
                }
            }
        } else {
            self.pressed = None;
        }
        self.gizmos.retain(|k, _| self.requests.iter().any(|r| r.kind == interact_kind::HANDLE && (r.ns.clone(), r.id.clone()) == *k));
        let aspect = viewport.width() / viewport.height().max(1.0);
        let clip_from_view = Mat4::perspective_rh(fov, aspect, near.max(0.01), 10_000.0);
        for r in self.requests.iter().filter(|r| r.kind == interact_kind::HANDLE) {
            let key = (r.ns.clone(), r.id.clone());
            let to_site = frame_to_site(&r.frame, &home);
            let world_pose = site.mul_transform(to_site * r.pose).compute_transform();
            let controls = r.props.get("controls").unwrap_or_else(|| "move,rotate".into());
            let mut modes = GizmoMode::all_translate() | GizmoMode::all_rotate();
            if !controls.contains("rotate") {
                modes = GizmoMode::all_translate();
            } else if !controls.contains("move") {
                modes = GizmoMode::all_rotate();
            }
            let (gizmo, was_focused) = self.gizmos.entry(key.clone()).or_insert_with(|| (GizmoCore::default(), false));
            gizmo.update_config(GizmoConfig {
                view_matrix: rows(Mat4::from(eye.affine().inverse())),
                projection_matrix: rows(clip_from_view),
                viewport: GizmoRect::from_x_y_ranges(viewport.min.x..=viewport.max.x, viewport.min.y..=viewport.max.y),
                modes,
                orientation: GizmoOrientation::Local,
                visuals: GizmoVisuals { gizmo_size: (75.0 * r.scale.min(2.0)).clamp(40.0, 150.0), ..Default::default() },
                pixels_per_point: egui.pixels_per_point(),
                ..Default::default()
            });
            let interaction = GizmoInteraction {
                cursor_pos: pos.map(|p| (p.x, p.y)).unwrap_or((-1.0, -1.0)),
                hovered: over,
                drag_started: pressed && over,
                dragging: down,
            };
            let t = GizmoTransform {
                scale: mint::Vector3 { x: 1.0, y: 1.0, z: 1.0 },
                rotation: mint::Quaternion {
                    v: mint::Vector3 { x: world_pose.rotation.x as f64, y: world_pose.rotation.y as f64, z: world_pose.rotation.z as f64 },
                    s: world_pose.rotation.w as f64,
                },
                translation: mint::Vector3 { x: world_pose.translation.x as f64, y: world_pose.translation.y as f64, z: world_pose.translation.z as f64 },
            };
            let moved = gizmo.update(interaction, &[t]).and_then(|(_, m)| m.first().copied());
            let focused = gizmo.is_focused();
            grab |= focused;
            let entity_pose = |wp: Transform| -> Transform {
                let local = GlobalTransform::from(to_site).affine().inverse() * site.affine().inverse() * wp.compute_affine();
                Transform::from_matrix(Mat4::from(local))
            };
            if focused && !*was_focused {
                let p = r.pose.translation;
                answer(world, r, "mouse_down", p, &[]);
            }
            if let Some(m) = moved {
                let wp = Transform {
                    translation: Vec3::new(m.translation.x as f32, m.translation.y as f32, m.translation.z as f32),
                    rotation: Quat::from_xyzw(m.rotation.v.x as f32, m.rotation.v.y as f32, m.rotation.v.z as f32, m.rotation.s as f32).normalize(),
                    scale: Vec3::ONE,
                };
                let ep = entity_pose(wp);
                let q = ep.rotation;
                answer(world, r, "pose_update", ep.translation, &[("qw", q.w.to_string()), ("qx", q.x.to_string()), ("qy", q.y.to_string()), ("qz", q.z.to_string())]);
                if let Some(store) = world.get_resource::<DrawStoreRes>() {
                    let p = ep.translation.as_dvec3();
                    let msg = DrawEntity::update(&r.ns, &r.id).with_pose([p.x, p.y, p.z], [q.w as f64, q.x as f64, q.y as f64, q.z as f64]).updating(&["pose"]);
                    let _ = store.lock().apply(msg, 0.0);
                }
            }
            if !focused && *was_focused {
                answer(world, r, "mouse_up", r.pose.translation, &[]);
            }
            *was_focused = focused;
            let centre = world_pose.translation;
            let screen = {
                let clip = clip_from_view * Mat4::from(eye.affine().inverse()) * centre.extend(1.0);
                (clip.w > 0.0).then(|| egui::pos2(viewport.min.x + (clip.x / clip.w * 0.5 + 0.5) * viewport.width(), viewport.min.y + (0.5 - clip.y / clip.w * 0.5) * viewport.height()))
            };
            if let (Some(s), Some(p)) = (screen, pos)
                && over
                && s.distance(p) < 14.0
            {
                if pressed && !focused && r.props.get("button").is_some() {
                    answer(world, r, "button_click", r.pose.translation, &[]);
                }
                if secondary && r.props.get("menu").is_some() {
                    self.menu = Some((r.ns.clone(), r.id.clone(), p));
                }
            }
        }
        if grab && let Some(mut g) = world.get_resource_mut::<crate::viewer::systems::GizmoGrab>() {
            g.0 = true;
        }
    }

    /// Remember the viewport and paint the step button, pick hints and
    /// handles over it.
    pub fn paint(&mut self, egui: &egui::Context, viewport: mara_core::vocab::Rect, world: &mut World) {
        let rect: egui::Rect = viewport.into();
        self.viewport = Some(rect);
        let painter = egui.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("gearbox_draw_interact"))).with_clip_rect(rect);
        let accent = egui::Color32::from_rgb(0x56, 0xB4, 0xE9);
        let goal = egui::Color32::from_rgb(0xF0, 0xE4, 0x42);
        let requests = self.requests.clone();
        let mut steps = 0;
        for r in &requests {
            match r.kind {
                interact_kind::STEP => {
                    let caption = r.props.get("text").unwrap_or_else(|| "Next".into());
                    let full = r.props.get("full").as_deref() == Some("1");
                    let at = egui::pos2(rect.center().x, rect.max.y - 54.0 - 46.0 * steps as f32);
                    steps += 1;
                    let mut clicked = false;
                    egui::Area::new(egui::Id::new(("gearbox_draw_step", &r.ns, &r.id)))
                        .order(egui::Order::Foreground)
                        .pivot(egui::Align2::CENTER_CENTER)
                        .fixed_pos(at)
                        .show(egui, |ui| {
                            egui::Frame::new().fill(PLATE).stroke(egui::Stroke::new(1.2, goal)).corner_radius(9.0).inner_margin(egui::Margin::symmetric(12, 7)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&caption).color(INK).size(13.0));
                                    let label = if full { "Continue ⏭" } else { "Next ▶" };
                                    clicked = ui.add(egui::Button::new(egui::RichText::new(label).color(egui::Color32::BLACK).strong()).fill(goal)).clicked();
                                });
                            });
                        });
                    let typing = egui.egui_wants_keyboard_input();
                    if clicked || (steps == 1 && !typing && egui.input(|i| i.key_pressed(egui::Key::N))) {
                        answer(world, r, "step", Vec3::ZERO, &[("full", if full { "1" } else { "0" }.into())]);
                        remove(world, r);
                    }
                }
                interact_kind::PICK_POINT | interact_kind::PICK_POSE => {
                    let text = r.props.get("text").unwrap_or_else(|| "Click a point".into());
                    let at = egui::pos2(rect.center().x, rect.min.y + 58.0);
                    let galley = painter.layout_no_wrap(format!("◎  {text}   ·   Esc cancels"), egui::FontId::proportional(13.0), INK);
                    let plate = egui::Rect::from_center_size(at, galley.size() + egui::vec2(28.0, 14.0));
                    painter.rect_filled(plate, 9.0, PLATE);
                    painter.rect_stroke(plate, 9.0, egui::Stroke::new(1.2, accent), egui::StrokeKind::Inside);
                    painter.galley(plate.center() - 0.5 * galley.size(), galley, INK);
                    if let Some(p) = self.hover {
                        let t = egui.input(|i| i.time) as f32;
                        let pulse = 9.0 + 3.0 * (t * 4.0).sin();
                        painter.circle_stroke(p, pulse, egui::Stroke::new(1.6, accent));
                        painter.circle_filled(p, 2.4, egui::Color32::WHITE);
                        egui.request_repaint();
                    }
                    if let (Some((from, _)), Some(to)) = (self.pressed, self.hover) {
                        let d = to - from;
                        if d.length() > 4.0 {
                            let n = d.normalized();
                            let side = egui::vec2(-n.y, n.x);
                            painter.line_segment([from, to], egui::Stroke::new(3.0, goal));
                            painter.add(egui::Shape::convex_polygon(vec![to + n * 10.0, to + side * 7.0, to - side * 7.0], goal, egui::Stroke::NONE));
                        }
                        painter.circle_filled(from, 5.0, goal);
                    }
                }
                _ => {}
            }
        }
        for ((_, _), (gizmo, _)) in self.gizmos.iter_mut() {
            let data = gizmo.draw();
            if data.vertices.is_empty() {
                continue;
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
            painter.add(egui::Shape::mesh(mesh));
        }
        if let Some((ns, id, at)) = self.menu.clone() {
            let Some(r) = requests.iter().find(|r| r.ns == ns && r.id == id).cloned() else {
                self.menu = None;
                return;
            };
            let entries: Vec<String> = r.props.get("menu").unwrap_or_default().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let mut picked = None;
            let mut close = false;
            egui::Area::new(egui::Id::new(("gearbox_draw_menu", &ns, &id))).order(egui::Order::Foreground).fixed_pos(at).show(egui, |ui| {
                egui::Frame::new().fill(PLATE).stroke(egui::Stroke::new(1.0, accent)).corner_radius(6.0).inner_margin(6.0).show(ui, |ui| {
                    for e in &entries {
                        if ui.button(egui::RichText::new(e).color(INK)).clicked() {
                            picked = Some(e.clone());
                        }
                    }
                    close = ui.input(|i| i.key_pressed(egui::Key::Escape));
                });
            });
            if let Some(entry) = picked {
                answer(world, &r, "menu_select", r.pose.translation, &[("menu", entry)]);
                self.menu = None;
            } else if close {
                self.menu = None;
            }
        }
    }
}
