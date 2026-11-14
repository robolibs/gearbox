//! Debug draws the host paints over the viewport: in-world charts (panels)
//! and the interactions clients ask for — a Next button, picking a point
//! or a pose on the ground, and 6-DoF handles — whose answers go out as
//! `draw` scene events.

use std::collections::HashMap;

use bevy::prelude::*;
use gearbox_api::wire::draw_geom::ramp;
use gearbox_api::{DrawEntity, DrawFrame, DrawStoreRes, GearboxBus, SceneEvent, colormap, draw_kind, event_kind, interact_kind, panel_kind};
use mara::ui::mara_core;
use mara::ui::modules::bevy as mara_bevy;
use transform_gizmo::math::Transform as GizmoTransform;
use transform_gizmo::config::Color32 as GizmoColor;
use transform_gizmo::{Gizmo as GizmoCore, GizmoConfig, GizmoInteraction, GizmoMode, GizmoOrientation, GizmoVisuals, Rect as GizmoRect, mint};

use crate::draw::{DrawPanel, PanelChart, PanelGlass, enu_to_site, pose_of, site_transform, visual};
#[cfg(test)]
use crate::draw::DrawPanels;
use super::draw_labels;

fn c32(c: [u8; 4], opacity: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (c[3] as f32 * opacity) as u8)
}

fn faded(c: egui::Color32, opacity: f32) -> egui::Color32 {
    c.gamma_multiply(opacity)
}

trait HolographicText {
    fn holographic_text(&self, at: egui::Pos2, align: egui::Align2, text: impl ToString, font: egui::FontId, color: egui::Color32) -> egui::Rect;
}

impl HolographicText for egui::Painter {
    fn holographic_text(&self, at: egui::Pos2, align: egui::Align2, text: impl ToString, font: egui::FontId, color: egui::Color32) -> egui::Rect {
        let galley = self.layout_no_wrap(text.to_string(), font, color);
        let rect = align.anchor_rect(egui::Rect::from_min_size(at, galley.size()));
        self.galley(rect.min, galley, color);
        rect
    }
}

fn panel_signal(painter: &egui::Painter, points: Vec<egui::Pos2>, width: f32, accent: egui::Color32, glow: f32, ink: egui::Color32) {
    if accent.a() == 0 || points.len() < 2 { return; }
    draw_labels::lit_path(painter, points.clone(), width + 1.5, accent, glow);
    painter.add(egui::Shape::line(points, egui::Stroke::new(width, faded(ink, accent.a() as f32 / 255.0))));
}

fn bar_body(tint: egui::Color32) -> egui::Color32 {
    tint.gamma_multiply(0.72)
}

const PANEL_WIDTH: f32 = 256.0;
const PAD: f32 = 14.0;
const HEADER: f32 = 36.0;

struct TableRow {
    key: std::sync::Arc<egui::Galley>,
    value: std::sync::Arc<egui::Galley>,
    height: f32,
}

struct TableLayout {
    rows: Vec<TableRow>,
}

impl TableLayout {
    fn prepare(painter: &egui::Painter, c: &PanelChart, opacity: f32) -> Option<Self> {
        if c.kind != panel_kind::TABLE { return None; }
        let rows: Vec<_> = if c.rows.is_empty() {
            c.labels.iter().zip(&c.values).map(|(key, value)| (key.clone(), format!("{} {}", number(*value), c.unit))).collect()
        } else { c.rows.clone() };
        let key_font = draw_labels::font(painter.ctx(), 14.0, true);
        let value_font = draw_labels::font(painter.ctx(), 15.0, true);
        let o = opacity * c.color[3] as f32 / 255.0;
        let dim = faded(draw_labels::glass_muted(c.glass.light()), o);
        let ink = faded(draw_labels::glass_ink(c.glass.light()), o);
        let available = PANEL_WIDTH - 2.0 * PAD - 12.0;
        let key_width = rows.iter().map(|(key, _)| painter.layout_no_wrap(key.clone(), key_font.clone(), dim).size().x)
            .fold(0.0_f32, f32::max).clamp(available * 0.25, available * 0.5).ceil();
        let rows = rows.into_iter().map(|(key, value)| {
            let key = painter.layout(key, key_font.clone(), dim, key_width);
            let mut job = egui::text::LayoutJob::simple(value, value_font.clone(), ink, available - key_width);
            job.halign = egui::Align::RIGHT;
            let value = painter.layout_job(job);
            let height = (key.size().y.max(value.size().y).ceil() + 10.0).max(30.0);
            TableRow { key, value, height }
        }).collect();
        Some(Self { rows })
    }

    fn height(&self) -> f32 {
        HEADER + self.rows.iter().map(|row| row.height).sum::<f32>().max(30.0) + 12.0
    }

    fn paint(&self, painter: &egui::Painter, r: egui::Rect, header: f32, ink: egui::Color32, dim: egui::Color32) {
        let mut y = r.top() + header;
        for row in &self.rows {
            let key_at = egui::pos2(r.left() + PAD - row.key.rect.left(), y + (row.height - row.key.size().y) * 0.5 - row.key.rect.top());
            let value_at = egui::pos2(r.right() - PAD - row.value.rect.right(), y + (row.height - row.value.size().y) * 0.5 - row.value.rect.top());
            painter.galley(key_at, row.key.clone(), dim);
            painter.galley(value_at, row.value.clone(), ink);
            y += row.height;
            painter.line_segment([egui::pos2(r.left() + PAD, y - 1.0), egui::pos2(r.right() - PAD, y - 1.0)],
                egui::Stroke::new(0.7, faded(dim, 0.3)));
        }
    }
}

struct PanelText {
    at: egui::Vec2,
    galley: std::sync::Arc<egui::Galley>,
    color: egui::Color32,
}

struct PanelContent {
    size: egui::Vec2,
    header: f32,
    text: Vec<PanelText>,
    table: Option<TableLayout>,
    area: egui::Rect,
    centre: egui::Pos2,
}

impl PanelContent {
    fn prepare(painter: &egui::Painter, c: &PanelChart, opacity: f32) -> Self {
        let o = opacity * c.color[3] as f32 / 255.0;
        let ink = faded(draw_labels::glass_ink(c.glass.light()), o);
        let dim = faded(draw_labels::glass_muted(c.glass.light()), o);
        let width = PANEL_WIDTH - 2.0 * PAD;
        let text = |value: String, size, color, width: f32, align| {
            let mut job = egui::text::LayoutJob::simple(value, draw_labels::font(painter.ctx(), size, true), color, width.max(1.0));
            job.halign = align;
            painter.layout_job(job)
        };
        let mut out = Self { size: egui::vec2(PANEL_WIDTH, 158.0), header: HEADER, text: Vec::new(),
            table: TableLayout::prepare(painter, c, opacity), area: egui::Rect::NOTHING, centre: egui::Pos2::ZERO };
        if c.kind != panel_kind::CHIP {
            let title = text(c.title.clone(), 16.0, ink, width, egui::Align::LEFT);
            out.header = HEADER.max(title.size().y + 17.0);
            out.push(egui::vec2(PAD, 10.0), egui::Align2::LEFT_TOP, title, ink);
        }
        let h = out.header;
        match c.kind {
            panel_kind::SPARKLINE => {
                let (lo, hi) = range(&c.values, false);
                let value = text(format!("{} {}", number(c.values.last().copied().unwrap_or(0.0)), c.unit), 24.0, ink, width, egui::Align::LEFT);
                let upper = text(number(hi), 12.0, dim, 80.0, egui::Align::RIGHT);
                let lower = text(number(lo), 12.0, dim, 80.0, egui::Align::RIGHT);
                let left = PAD + 35.0_f32.max(upper.size().x.max(lower.size().x) + 7.0);
                let top = h + 46.0_f32.max(4.0 + value.size().y + 7.0 + upper.size().y * 0.5);
                let height = 60.0_f32.max((upper.size().y + lower.size().y) * 0.5 + 8.0);
                out.area = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(PANEL_WIDTH - PAD, top + height));
                out.size.y = out.area.bottom() + 16.0_f32.max(lower.size().y * 0.5 + 6.0);
                out.push(egui::vec2(PAD, h + 4.0), egui::Align2::LEFT_TOP, value, ink);
                out.push(egui::vec2(left - 7.0, top), egui::Align2::RIGHT_CENTER, upper, dim);
                out.push(egui::vec2(left - 7.0, top + height), egui::Align2::RIGHT_CENTER, lower, dim);
            }
            panel_kind::BARS => {
                let slot = width / c.values.len().max(1) as f32;
                let mut values = Vec::new();
                let mut labels = Vec::new();
                for (i, value) in c.values.iter().enumerate().filter(|(_, v)| v.is_finite()) {
                    values.push((i, text(number(*value), 14.0, ink, slot - 8.0, egui::Align::Center)));
                    if let Some(label) = c.labels.get(i) { labels.push((i, text(label.clone(), 13.0, dim, slot - 8.0, egui::Align::Center))); }
                }
                let top = h + values.iter().map(|(_, text)| text.size().y + 7.0).fold(28.0, f32::max);
                let bottom = labels.iter().map(|(_, text)| text.size().y + 14.0).fold(27.0, f32::max);
                out.area = egui::Rect::from_min_size(egui::pos2(PAD, top), egui::vec2(width, 67.0));
                out.size.y = out.area.bottom() + bottom;
                for (i, value) in values { out.push(egui::vec2(PAD + slot * (i as f32 + 0.5), top - 7.0), egui::Align2::CENTER_BOTTOM, value, ink); }
                for (i, label) in labels { out.push(egui::vec2(PAD + slot * (i as f32 + 0.5), out.area.bottom() + 7.0), egui::Align2::CENTER_TOP, label, dim); }
            }
            panel_kind::GAUGE => {
                let value = text(number(c.values.first().copied().unwrap_or(0.0)), 28.0, ink, PANEL_WIDTH - PAD - 132.0, egui::Align::LEFT);
                let unit = text(c.unit.clone(), 14.0, dim, PANEL_WIDTH - PAD - 134.0, egui::Align::LEFT);
                let limits = text(format!("{} — {}", number(c.min), number(c.max)), 12.0, dim, 112.0, egui::Align::Center);
                let height = value.size().y + 4.0 + unit.size().y;
                out.centre = egui::pos2(70.0, h + 60.0_f32.max(height * 0.5 + 14.0));
                let top = out.centre.y - height * 0.5;
                out.size.y = (h + 122.0).max(top + height + PAD).max(out.centre.y + 27.0 + 10.0 + limits.size().y + 11.0);
                let unit_y = top + value.size().y + 4.0;
                out.push(egui::vec2(132.0, top), egui::Align2::LEFT_TOP, value, ink);
                out.push(egui::vec2(134.0, unit_y), egui::Align2::LEFT_TOP, unit, dim);
                out.push(egui::vec2(70.0, out.size.y - 11.0), egui::Align2::CENTER_BOTTOM, limits, dim);
            }
            panel_kind::CHIP => {
                let label = text(if c.text.is_empty() { c.title.clone() } else { c.text.clone() }, 16.0, ink, PANEL_WIDTH - PAD - 54.0, egui::Align::LEFT);
                out.size.y = 58.0_f32.max(label.size().y + 28.0);
                out.push(egui::vec2(54.0, out.size.y * 0.5), egui::Align2::LEFT_CENTER, label, ink);
            }
            panel_kind::TABLE => { out.size.y = h + out.table.as_ref().map_or(42.0, |table| table.height() - HEADER); }
            panel_kind::LEGEND => {
                out.area = egui::Rect::from_min_size(egui::pos2(PAD, h + 12.0), egui::vec2(width, 17.0));
                out.size.y = h + 72.0;
                for (t, v) in [(0.0, c.map.1), (0.5, 0.5 * (c.map.1 + c.map.2)), (1.0, c.map.2)] {
                    let (align, halign) = if t == 0.0 { (egui::Align2::LEFT_TOP, egui::Align::LEFT) }
                        else if t == 1.0 { (egui::Align2::RIGHT_TOP, egui::Align::RIGHT) } else { (egui::Align2::CENTER_TOP, egui::Align::Center) };
                    let label = text(format!("{}{}", number(v), if t == 1.0 { format!(" {}", c.unit) } else { String::new() }), 13.0, ink, width / 3.0 - 8.0, halign);
                    let top = out.area.bottom() + 12.0;
                    out.size.y = out.size.y.max(top + label.size().y + PAD);
                    out.push(egui::vec2(PAD + width * t, top), align, label, ink);
                }
            }
            _ => {}
        }
        out
    }

    fn push(&mut self, at: egui::Vec2, align: egui::Align2, galley: std::sync::Arc<egui::Galley>, color: egui::Color32) {
        let rect = align.anchor_rect(egui::Rect::from_min_size(at.to_pos2(), galley.size()));
        self.text.push(PanelText { at: rect.min.to_vec2() - galley.rect.min.to_vec2(), galley, color });
    }

    fn paint(&self, painter: &egui::Painter, r: egui::Rect, c: &PanelChart, opacity: f32) {
        for text in &self.text { painter.galley(r.min + text.at, text.galley.clone(), text.color); }
        if let Some(table) = &self.table {
            let o = opacity * c.color[3] as f32 / 255.0;
            table.paint(painter, r, self.header, faded(draw_labels::glass_ink(c.glass.light()), o), faded(draw_labels::glass_muted(c.glass.light()), o));
        }
    }
}

fn panel_rect(anchor: egui::Pos2, size: egui::Vec2) -> egui::Rect {
    egui::Rect::from_min_size(anchor - egui::vec2(size.x * 0.5, size.y + 28.0), size)
}

fn panel_placement(bounds: egui::Rect, viewport: egui::Rect, occupied: &[egui::Rect]) -> egui::Rect {
    super::draw_layout::place(bounds, viewport, occupied, 16.0)
}

pub(super) struct PanelLayout<'a> {
    panel: &'a DrawPanel,
    anchor: Option<egui::Pos2>,
    bounds: egui::Rect,
    content: PanelContent,
}

pub(super) fn prepare_panels<'a>(egui: &egui::Context, rect: egui::Rect, panels: &'a [DrawPanel], occupied: &mut Vec<egui::Rect>) -> Vec<PanelLayout<'a>> {
    let painter = egui.layer_painter(egui::LayerId::background());
    let mut layouts = Vec::new();
    let mut legend_y = rect.max.y - 18.0;
    for panel in panels.iter().filter(|p| p.at.is_none()).chain(panels.iter().filter(|p| p.at.is_some())) {
        if !panel.opacity.is_finite() || panel.opacity <= 0.0 || panel.chart.color[3] == 0
            || panel.at.is_some_and(|at| at.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))) { continue; }
        let content = PanelContent::prepare(&painter, &panel.chart, panel.opacity);
        let size = content.size;
        match panel.at {
            None => {
                let initial = egui::Rect::from_min_size(egui::pos2(rect.right() - size.x - 18.0, legend_y - size.y), size);
                let r = panel_placement(initial, rect, occupied);
                layouts.push(PanelLayout { panel, anchor: None, bounds: r, content });
                occupied.push(r);
                legend_y -= size.y + 14.0;
            }
            Some(at) => {
                let anchor = rect.min + egui::vec2(at[0] * rect.width(), at[1] * rect.height());
                let r = panel_placement(panel_rect(anchor, size), rect, occupied);
                occupied.push(r);
                layouts.push(PanelLayout { panel, anchor: Some(anchor), bounds: r, content });
            }
        }
    }
    layouts
}

pub(super) fn paint_panels(egui: &egui::Context, viewport: egui::Rect, layouts: &[PanelLayout<'_>], occupied: &[egui::Rect]) {
    let painter = egui.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("gearbox_draw_panels"))).with_clip_rect(viewport);
    for layout in layouts {
        match layout.anchor {
            Some(anchor) => chart_at(&painter, anchor, layout.bounds, layout.panel, &layout.content, occupied),
            None => legend(&painter, layout.bounds, &layout.panel.chart, layout.panel.opacity, &layout.content),
        }
    }
}

#[cfg(test)]
fn paint_draw_panels(egui: &egui::Context, viewport: mara_core::vocab::Rect, world: &World, occupied: &mut Vec<egui::Rect>) {
    let panels = world.get_resource::<DrawPanels>().map_or(&[][..], |panels| panels.0.as_slice());
    let layouts = prepare_panels(egui, viewport.into(), panels, occupied);
    paint_panels(egui, viewport.into(), &layouts, occupied);
}

fn heading(painter: &egui::Painter, r: egui::Rect, c: &PanelChart, accent: egui::Color32, opacity: f32, header: f32) {
    let dim = draw_labels::glass_muted(c.glass.light());
    let painter = painter.with_clip_rect(r.shrink(4.0));
    let y = r.top() + header - 3.0;
    painter.line_segment([egui::pos2(r.left() + PAD, y), egui::pos2(r.right() - PAD, y)],
        egui::Stroke::new(1.0, faded(dim, opacity * 0.35)));
    draw_labels::lit_path(&painter, vec![egui::pos2(r.left() + PAD, y), egui::pos2(r.left() + PAD + 28.0, y)],
        1.4, faded(accent, opacity), c.glow);
}

fn number(v: f32) -> String {
    if !v.is_finite() { "—".into() } else if v.abs() >= 100.0 { format!("{v:.0}") }
    else if v.abs() >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") }
}

fn bar_tint(c: &PanelChart, index: usize) -> [u8; 4] {
    c.colors.get(index).copied().filter(|color| *color != [0; 4]).unwrap_or([c.color[0], c.color[1], c.color[2], 255])
}

fn range(values: &[f32], include_zero: bool) -> (f32, f32) {
    let mut lo = if include_zero { 0.0 } else { f32::INFINITY };
    let mut hi = if include_zero { 0.0 } else { f32::NEG_INFINITY };
    for &v in values.iter().filter(|v| v.is_finite()) { lo = lo.min(v); hi = hi.max(v); }
    if !lo.is_finite() || !hi.is_finite() { return (0.0, 1.0); }
    if (hi - lo).abs() < 1e-6 { let pad = (lo.abs() * 0.1).max(0.5); (lo - pad, hi + pad) } else { (lo, hi) }
}

#[cfg(test)]
fn chart(painter: &egui::Painter, anchor: egui::Pos2, panel: &DrawPanel) {
    let content = PanelContent::prepare(painter, &panel.chart, panel.opacity);
    chart_at(painter, anchor, panel_rect(anchor, content.size), panel, &content, &[]);
}

fn chart_at(painter: &egui::Painter, anchor: egui::Pos2, r: egui::Rect, panel: &DrawPanel, content: &PanelContent, occupied: &[egui::Rect]) {
    let c = &panel.chart;
    let ink = draw_labels::glass_ink(c.glass.light());
    let dim = draw_labels::glass_muted(c.glass.light());
    let o = panel.opacity * c.color[3] as f32 / 255.0;
    if o <= 0.0 { return; }
    let accent = c32([c.color[0], c.color[1], c.color[2], 255], 1.0);
    if !c.badge {
        draw_labels::leader(painter, anchor, r, faded(accent, o), c.glow, occupied);
    }
    if c.kind == panel_kind::LEGEND { legend(painter, r, c, panel.opacity, content); return; }
    draw_labels::glass_frame(painter, r, faded(accent, o), false, c.glow, c.glass, c.blur);
    if c.kind != panel_kind::CHIP { heading(painter, r, c, accent, o, content.header); }
    let painter = painter.with_clip_rect(r.shrink(6.0));
    match c.kind {
        panel_kind::SPARKLINE => {
            let (lo, hi) = range(&c.values, false);
            let area = content.area.translate(r.min.to_vec2());
            for t in [0.0, 0.5, 1.0] {
                let y = area.bottom() - t * area.height();
                painter.line_segment([egui::pos2(area.left(), y), egui::pos2(area.right(), y)],
                    egui::Stroke::new(0.7, faded(dim, o * 0.3)));
            }
            let n = c.values.len().saturating_sub(1).max(1) as f32;
            let mut path = Vec::new();
            for (i, &value) in c.values.iter().enumerate() {
                if !value.is_finite() {
                    panel_signal(&painter, std::mem::take(&mut path), 1.6, faded(accent, o), c.glow, ink);
                    continue;
                }
                path.push(egui::pos2(area.left() + area.width() * i as f32 / n, area.bottom() - area.height() * (value - lo) / (hi - lo)));
            }
            let end = path.last().copied();
            panel_signal(&painter, path, 1.6, faded(accent, o), c.glow, ink);
            if let Some(end) = end { painter.circle_filled(end, 2.2, faded(ink, o)); }
        }
        panel_kind::BARS => {
            let (lo, mut hi) = range(&c.values, true);
            if c.max_given && c.max.is_finite() { hi = hi.max(c.max); }
            let area = content.area.translate(r.min.to_vec2());
            let zero = area.bottom() - area.height() * (-lo / (hi - lo));
            painter.line_segment([egui::pos2(area.left(), zero), egui::pos2(area.right(), zero)], egui::Stroke::new(1.0, faded(dim, o * 0.6)));
            let slot = area.width() / c.values.len().max(1) as f32;
            for (i, &v) in c.values.iter().enumerate().filter(|(_, v)| v.is_finite()) {
                let col = c32(bar_tint(c, i), o);
                let y = area.bottom() - area.height() * ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
                let bar = egui::Rect::from_min_max(egui::pos2(area.left() + slot * (i as f32 + 0.18), y.min(zero)),
                    egui::pos2(area.left() + slot * (i as f32 + 0.82), y.max(zero)));
                painter.rect_filled(bar, 0.0, bar_body(col));
                painter.rect_stroke(bar, 0.0, egui::Stroke::new(0.8, faded(ink, col.a() as f32 / 255.0 * 0.55)), egui::StrokeKind::Inside);
                panel_signal(&painter, vec![egui::pos2(bar.left(), y), egui::pos2(bar.right(), y)], 1.0, col, c.glow, ink);
            }
        }
        panel_kind::GAUGE => {
            let centre = content.centre + r.min.to_vec2();
            let radius = 38.0;
            let value = c.values.first().copied().unwrap_or(0.0);
            let share = ((value - c.min) / (c.max - c.min).max(1e-6)).clamp(0.0, 1.0);
            let start = std::f32::consts::PI * 0.75;
            let sweep = std::f32::consts::PI * 1.5;
            let arc = |end: f32| (0..=48).map(|i| { let a = start + sweep * end * i as f32 / 48.0; centre + radius * egui::vec2(a.cos(), a.sin()) }).collect::<Vec<_>>();
            draw_labels::lit_path(&painter, arc(1.0), 3.0, faded(dim, o * 0.25), 0.0);
            if share.is_finite() && share > 0.0 { panel_signal(&painter, arc(share), 3.0, faded(accent, o), c.glow, ink); }
            for i in 0..=20 {
                let a = start + sweep * i as f32 / 20.0;
                let d = egui::vec2(a.cos(), a.sin());
                painter.line_segment([centre + d * (radius + 5.0), centre + d * (radius + if i % 5 == 0 { 10.0 } else { 7.0 })],
                    egui::Stroke::new(1.0, faded(dim, o * 0.7)));
            }
        }
        panel_kind::CHIP => {
            draw_labels::anchor(&painter, egui::pos2(r.left() + 25.0, r.center().y), faded(accent, o));
            painter.line_segment([egui::pos2(r.left() + 43.0, r.top() + 14.0), egui::pos2(r.left() + 43.0, r.bottom() - 14.0)],
                egui::Stroke::new(1.0, faded(dim, o * 0.4)));
        }
        _ => {}
    }
    content.paint(&painter, r, c, panel.opacity);
}

fn legend(painter: &egui::Painter, r: egui::Rect, c: &PanelChart, opacity: f32, content: &PanelContent) {
    let dim = draw_labels::glass_muted(c.glass.light());
    let o = opacity * c.color[3] as f32 / 255.0;
    if o <= 0.0 { return; }
    let accent = c32([c.color[0], c.color[1], c.color[2], 255], 1.0);
    draw_labels::glass_frame(painter, r, faded(accent, o), false, c.glow, c.glass, c.blur);
    heading(painter, r, c, accent, o, content.header);
    let painter = painter.with_clip_rect(r.shrink(6.0));
    let bar = content.area.translate(r.min.to_vec2());
    let (map, lo, hi) = (if c.map.0 == colormap::NONE { colormap::VIRIDIS } else { c.map.0 }, c.map.1, c.map.2);
    let steps = 64;
    for k in 0..steps {
        let t0 = k as f32 / steps as f32;
        let t1 = (k + 1) as f32 / steps as f32;
        let col = if colormap::is_palette(map) { gearbox_api::wire::draw_geom::palette(map, lo + (hi - lo) * t0) }
            else { ramp(map, if lo > hi { 1.0 - t0 } else { t0 }) };
        painter.rect_filled(egui::Rect::from_min_max(egui::pos2(bar.left() + bar.width() * t0, bar.top()),
            egui::pos2(bar.left() + bar.width() * t1 + 0.5, bar.bottom())), 0.0, c32(col, o));
    }
    for t in [0.0, 0.5, 1.0] {
        let x = bar.left() + bar.width() * t;
        painter.line_segment([egui::pos2(x, bar.bottom() + 2.0), egui::pos2(x, bar.bottom() + 7.0)], egui::Stroke::new(1.0, faded(dim, o)));
    }
    content.paint(&painter, r, c, opacity);
}

#[cfg(test)]
mod drawing_tests {
    use super::*;

    #[test]
    fn interaction_skin_preserves_tint_alpha_finish_and_glow() {
        for kind in [interact_kind::STEP, interact_kind::PICK_POINT, interact_kind::PICK_POSE, interact_kind::HANDLE] {
            for rgba in [[216, 72, 145, 255], [64, 175, 110, 128], [1, 0, 0, 0]] {
                for (finish, glass) in [("white", PanelGlass::White), ("black", PanelGlass::Black), ("color", PanelGlass::Color { light: false })] {
                    let e = DrawEntity::interact("test", "control", kind).with_color(u32::from_le_bytes(rgba))
                        .with_prop("panel_finish", finish).with_glow(-1.0);
                    let skin = ControlSkin::of(&e);
                    assert_eq!(skin.accent, c32(rgba, 1.0));
                    assert_eq!(skin.glass, glass);
                    assert_eq!(skin.glow, 0.0);
                    assert_eq!(skin.ink().a(), rgba[3]);
                    assert_eq!(skin.muted().a(), rgba[3]);
                }
            }
        }
        let auto = ControlSkin::of(&DrawEntity::interact("test", "control", interact_kind::STEP));
        assert_eq!(auto.accent, c32(visual::ACCENT, 1.0));
        assert_eq!(auto.glass, PanelGlass::Black);
        assert_eq!(auto.glow, 0.65);
    }

    #[test]
    fn glass_control_button_keeps_label_and_click_target() {
        let ctx = egui::Context::default();
        let skin = ControlSkin::of(&DrawEntity::interact("test", "step", interact_kind::STEP)
            .with_color(0xffa749cc).with_prop("panel_finish", "white"));
        let target = egui::Rect::from_min_size(egui::pos2(40.0, 40.0), egui::vec2(140.0, 32.0));
        let mut clicked = false;
        for events in [vec![], vec![], vec![egui::Event::PointerMoved(target.center()), egui::Event::PointerButton {
            pos: target.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE,
        }], vec![egui::Event::PointerButton {
            pos: target.center(), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE,
        }]] {
            let output = crate::host::run_ui_clean(&ctx, egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 300.0))), events, ..Default::default() }, |ui| {
                let response = skin.button(ui, target, "Inspect robot", egui::Id::new("control-test"));
                assert_eq!(response.rect, target);
                clicked |= response.clicked();
            });
            assert_eq!(output.shapes.iter().filter(|shape| matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.text() == "Inspect robot" && text.fallback_color == skin.ink())).count(), 1);
        }
        assert!(clicked);
    }

    #[test]
    fn handle_drag_requires_a_press_not_just_hover() {
        assert!(!handle_dragging(false, true, false, false, true));
        assert!(!handle_dragging(false, true, false, true, true));
        assert!(!handle_dragging(false, true, true, true, false));
        assert!(!handle_dragging(false, false, true, true, true));
        assert!(handle_dragging(false, true, true, true, true));
        assert!(handle_dragging(true, false, false, true, false));
        assert!(!handle_dragging(true, true, false, false, true));
    }

    #[test]
    fn standalone_controls_request_frost_only_when_visible() {
        let mut controls = DrawInteract::default();
        assert!(!controls.needs_frost());
        for kind in [interact_kind::STEP, interact_kind::PICK_POINT, interact_kind::PICK_POSE, interact_kind::HANDLE] {
            for color in [0xffa749cc, 1] {
                let e = DrawEntity::interact("test", "control", kind).with_color(color);
                controls.requests = vec![Request { ns: "test".into(), id: "control".into(), kind, frame: e.frame(),
                    pose: pose_of(&e), scale: 1.0, skin: ControlSkin::of(&e), props: e.props() }];
                controls.menu = None;
                assert_eq!(controls.needs_frost(), color != 1 && kind != interact_kind::HANDLE);
                controls.menu = Some(("test".into(), "control".into(), egui::Pos2::ZERO));
                assert_eq!(controls.needs_frost(), color != 1);
            }
        }
    }

    #[test]
    fn offscreen_panel_anchors_do_not_become_screen_edge_indicators() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut world = World::default();
        for at in [[-0.1, 0.5], [1.1, 0.5], [0.5, -0.1], [0.5, 1.1], [f32::NAN, 0.5], [0.5, f32::INFINITY]] {
            world.insert_resource(DrawPanels(vec![DrawPanel { key: ("t".into(), "p".into()), chart: PanelChart {
                kind: panel_kind::CHIP, color: [100, 200, 250, 255], title: "Hidden".into(), ..Default::default()
            }, at: Some(at), opacity: 1.0 }]));
            let mut occupied = Vec::new();
            let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| paint_draw_panels(ui.ctx(), viewport.into(), &world, &mut occupied));
            assert!(occupied.is_empty());
            assert!(output.shapes.iter().all(|shape| matches!(shape.shape, egui::Shape::Noop)));
        }
    }

    #[test]
    fn panel_near_the_top_edge_uses_available_viewport_space() {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 200.0));
        let bounds = panel_rect(egui::pos2(250.0, 10.0), egui::vec2(PANEL_WIDTH, 158.0));
        let placed = panel_placement(bounds, viewport, &[]);
        assert!(viewport.shrink(8.0).contains_rect(placed), "{placed:?}");
    }

    #[test]
    fn displaced_panel_leader_does_not_cross_its_contents() {
        let ctx = egui::Context::default();
        let anchor = egui::pos2(250.0, 50.0);
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 200.0), egui::vec2(256.0, 158.0));
        let panel = DrawPanel { key: ("t".into(), "p".into()), chart: PanelChart {
            kind: panel_kind::CHIP, color: [100, 200, 250, 255], title: "Status".into(), glow: 0.0, ..Default::default()
        }, at: Some([0.5, 0.5]), opacity: 1.0 };
        let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
            let content = PanelContent::prepare(ui.painter(), &panel.chart, panel.opacity);
            chart_at(ui.painter(), anchor, rect, &panel, &content, &[]);
        });
        let leader = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Path(path) if path.points.first() == Some(&anchor) => Some(path),
            _ => None,
        }).unwrap();
        for segment in leader.points.windows(2) {
            for i in 1..100 {
                let point = segment[0].lerp(segment[1], i as f32 / 100.0);
                assert!(!rect.shrink(0.1).contains(point), "leader crosses content at {point:?}");
            }
        }
    }

    #[test]
    fn panel_families_share_frame_width_and_separate_bounds() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0));
        let mut occupied = Vec::new();
        let _ = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
            for kind in 0..=panel_kind::LEGEND {
                let data = PanelChart { kind, rows: vec![("Source".into(), "Fixture".into()); 3], ..Default::default() };
                let size = PanelContent::prepare(ui.painter(), &data, 1.0).size;
                assert_eq!(size.x, PANEL_WIDTH);
                let initial = panel_rect(egui::pos2(600.0, 450.0), size);
                let placed = panel_placement(initial, viewport, &occupied);
                assert!(viewport.contains_rect(placed));
                assert!(occupied.iter().all(|other| !placed.intersects(*other)));
                occupied.push(placed);
            }
        });
    }

    #[test]
    fn narrow_panels_fit_beside_each_other_below_reserved_chrome() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(640.0, 400.0));
        let chrome = egui::Rect::from_min_size(viewport.min + egui::vec2(80.0, 8.0), egui::vec2(480.0, 28.0));
        for scale in [1.0, 1.5, 2.0] {
            for glass in [PanelGlass::White, PanelGlass::Black, PanelGlass::Color { light: false }] {
                let mut world = World::default();
                world.insert_resource(DrawPanels([0.75, 0.55].into_iter().enumerate().map(|(i, x)| DrawPanel {
                    key: ("slots".into(), i.to_string()), at: Some([x, 0.7]), opacity: 1.0,
                    chart: PanelChart { kind: panel_kind::TABLE, title: "Inspection".into(), glass,
                        color: [90, 180, 220, 255], rows: vec![("Sample".into(), "027".into()); 8], ..Default::default() },
                }).collect()));
                for _ in 0..2 {
                    let mut input = egui::RawInput { screen_rect: Some(viewport), ..Default::default() };
                    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
                    let _ = crate::host::run_ui_clean(&ctx, input, |ui| {
                        let mut occupied = vec![chrome];
                        let panels = prepare_panels(ui.ctx(), viewport, &world.resource::<DrawPanels>().0, &mut occupied);
                        assert_eq!(panels.len(), 2);
                        for (i, panel) in panels.iter().enumerate() {
                            assert!((panel.bounds.size() - panel.content.size).length() < 0.001);
                            assert!((panel.bounds.width() - PANEL_WIDTH).abs() < 0.001);
                            assert!(viewport.shrink(8.0).contains_rect(panel.bounds));
                            assert_eq!(panel.anchor, Some(viewport.min + egui::vec2([0.75, 0.55][i] * viewport.width(), 0.7 * viewport.height())));
                            assert!(occupied.iter().filter(|&&r| r != panel.bounds).all(|other| !panel.bounds.expand(7.99).intersects(*other)));
                        }
                    });
                }
            }
        }
    }

    #[test]
    fn chart_ranges_preserve_signed_constant_and_missing_samples() {
        assert_eq!(range(&[-2.0, 3.0], true), (-2.0, 3.0));
        assert_eq!(range(&[f32::NAN], false), (0.0, 1.0));
        let (lo, hi) = range(&[0.65; 3], false);
        assert!(lo < 0.65 && hi > 0.65 && hi - lo > 0.0);
        assert_eq!(number(f32::NAN), "—");
    }

    #[test]
    fn panel_content_stays_complete_and_separate_across_families() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 1000.0));
        for scale in [1.0, 1.5, 2.0] {
            for kind in 0..=panel_kind::LEGEND {
                for glass in [PanelGlass::White, PanelGlass::Black, PanelGlass::Color { light: false }] {
                    let data = PanelChart { kind, title: "Local motion / measured longitudinal velocity".into(),
                        text: "Waiting for a valid localization estimate / α β".into(),
                        values: vec![-1234.5, 0.65, 9876.0],
                        labels: vec!["Forward velocity".into(), "Tracking error".into(), "Reference velocity".into()],
                        rows: vec![("Coordinate frame".into(), "machine / odometry".into())],
                        unit: "millimetres / second".into(), min: -2000.0, max: 10000.0,
                        map: (colormap::VIRIDIS, -2000.0, 10000.0), color: [204, 121, 167, 192], glass,
                        ..Default::default() };
                    let mut expected = vec![if kind == panel_kind::CHIP { data.text.clone() } else { data.title.clone() }];
                    expected.extend(match kind {
                        panel_kind::SPARKLINE => vec!["9876 millimetres / second", "9876", "-1234"],
                        panel_kind::BARS => vec!["-1234", "0.65", "9876", "Forward velocity", "Tracking error", "Reference velocity"],
                        panel_kind::GAUGE => vec!["-1234", "millimetres / second", "-2000 — 10000"],
                        panel_kind::TABLE => vec!["Coordinate frame", "machine / odometry"],
                        panel_kind::LEGEND => vec!["-2000", "4000", "10000 millimetres / second"],
                        _ => vec![],
                    }.into_iter().map(str::to_owned));
                    let mut world = World::default();
                    world.insert_resource(DrawPanels(vec![DrawPanel { key: ("test".into(), "content".into()),
                        at: Some([0.5, 0.8]), chart: data, opacity: 0.75 }]));
                    for _ in 0..3 {
                        let mut occupied = Vec::new();
                        let mut input = egui::RawInput { screen_rect: Some(viewport), ..Default::default() };
                        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
                        let output = crate::host::run_ui_clean(&ctx, input, |ui| {
                            assert_eq!(ui.ctx().pixels_per_point(), scale);
                            paint_draw_panels(ui.ctx(), viewport.into(), &world, &mut occupied);
                        });
                        let mut bounds = Vec::new();
                        let mut found = Vec::new();
                        for shape in &output.shapes {
                            let egui::Shape::Text(text) = &shape.shape else { continue; };
                            found.push(text.galley.text());
                            let rect = text.galley.rect.translate(text.pos.to_vec2());
                            assert!(!text.galley.elided, "kind {kind}: elided {}", text.galley.text());
                            assert!(shape.clip_rect.expand(0.5).contains_rect(rect), "kind {kind}, scale {scale}: clipped {}: {rect:?} / {:?}", text.galley.text(), shape.clip_rect);
                            assert!(occupied[0].shrink(6.0).expand(0.5).contains_rect(rect), "kind {kind}: outside {}", text.galley.text());
                            assert!(bounds.iter().all(|other| !rect.intersects(*other)), "kind {kind}: overlapping {}", text.galley.text());
                            bounds.push(rect);
                        }
                        for text in &expected { assert!(found.contains(&text.as_str()), "kind {kind}: missing {text}"); }
                        assert!(viewport.contains_rect(occupied[0]));
                    }
                }
            }
        }
    }

    #[test]
    fn short_panel_content_retains_compact_frames_and_data_geometry() {
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let _ = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
                for kind in 0..=panel_kind::LEGEND {
                    let data = PanelChart { kind, title: "Speed".into(), text: "Ready".into(), unit: "m/s".into(),
                        values: vec![-0.3, 0.2, 0.8], labels: vec!["X".into(), "Y".into(), "Z".into()],
                        min: 0.0, max: 1.0, map: (colormap::VIRIDIS, 0.0, 1.0), color: visual::ACCENT, ..Default::default() };
                    let content = PanelContent::prepare(ui.painter(), &data, 1.0);
                    assert_eq!(content.size.x, PANEL_WIDTH);
                    assert!((58.0..=174.0).contains(&content.size.y), "kind {kind}: {:?}", content.size);
                    assert!((HEADER..=HEADER + 1.0).contains(&content.header));
                    if kind == panel_kind::CHIP { assert_eq!(content.size.y, 58.0); }
                    if kind == panel_kind::BARS {
                        assert_eq!(content.area.height(), 67.0);
                        let labels = &content.text[4..];
                        for (i, label) in labels.iter().enumerate() {
                            let bounds = label.galley.rect.translate(label.at);
                            let expected = content.area.left() + content.area.width() * (i as f32 + 0.5) / 3.0;
                            assert!((bounds.center().x - expected).abs() < 0.01);
                        }
                    }
                    if kind == panel_kind::SPARKLINE { assert_eq!(content.area.height(), 60.0); }
                }
            });
        }
    }

    #[test]
    fn table_cells_keep_complete_strings_inside_their_painted_bounds() {
        let ctx = egui::Context::default();
        let rows = vec![
            ("Source".into(), "Deterministic fixture".into()),
            ("Grid".into(), "0–100 / unknown 255".into()),
            ("Velocity along local X".into(), "−1234.50 mm/s".into()),
            ("Long identifier".into(), "sensor_0123456789_abcdefghijklmnopqrstuvwxyz".into()),
            ("Notes".into(), "Line one\nLine two / α β".into()),
        ];
        for scale in [1.0, 1.5, 2.0] {
            for glass in [PanelGlass::White, PanelGlass::Black, PanelGlass::Color { light: false }] {
                let panel = DrawPanel { key: ("test".into(), "table".into()), at: Some([0.5, 0.7]), opacity: 1.0,
                    chart: PanelChart { kind: panel_kind::TABLE, rows: rows.clone(), title: "Inspection".into(),
                        color: visual::ACCENT, glass, ..Default::default() } };
                let mut world = World::default();
                world.insert_resource(DrawPanels(vec![panel]));
                let mut occupied = Vec::new();
                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 900.0));
                let mut input = egui::RawInput { screen_rect: Some(viewport), ..Default::default() };
                input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
                let output = crate::host::run_ui_clean(&ctx, input, |ui| {
                    assert_eq!(ui.ctx().pixels_per_point(), scale);
                    paint_draw_panels(ui.ctx(), viewport.into(), &world, &mut occupied);
                });
                let mut bounds = Vec::new();
                for content in rows.iter().flat_map(|(key, value)| [key, value]) {
                    let (clipped, text) = output.shapes.iter().find_map(|s| match &s.shape {
                        egui::Shape::Text(text) if text.galley.text() == content => Some((s, text)), _ => None,
                    }).unwrap();
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    assert!(!text.galley.elided, "elided {content}");
                    assert!(clipped.clip_rect.expand(0.5).contains_rect(rect), "clipped {content}: {rect:?} / {:?}", clipped.clip_rect);
                    assert!(occupied.iter().any(|panel| panel.shrink(6.0).contains_rect(rect)));
                    assert!(bounds.iter().all(|other| !rect.intersects(*other)), "overlapping {content}");
                    bounds.push(rect);
                }
                assert!(occupied.iter().all(|panel| viewport.contains_rect(*panel)));
            }
        }
    }

    #[test]
    fn table_numeric_rows_keep_signs_units_and_empty_tables() {
        let ctx = egui::Context::default();
        let _ = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
            let data = PanelChart { kind: panel_kind::TABLE, values: vec![-1234.5, 0.0, 0.65],
                labels: vec!["Velocity along local X".into(), "Stopped".into(), "Tracking error".into()],
                unit: "mm/s".into(), color: visual::ACCENT, ..Default::default() };
            let table = TableLayout::prepare(ui.painter(), &data, 0.5).unwrap();
            assert_eq!(table.rows.len(), 3);
            for ((key, value), row) in data.labels.iter().zip(&data.values).zip(&table.rows) {
                assert_eq!(row.key.text(), key);
                assert_eq!(row.value.text(), format!("{} mm/s", number(*value)));
                assert!(!row.key.elided && !row.value.elided);
                assert!(row.height >= row.key.size().y.max(row.value.size().y) + 10.0);
                assert_eq!(row.value.job.sections[0].format.color, faded(draw_labels::GLASS_INK, 0.5));
            }
            let empty = TableLayout::prepare(ui.painter(), &PanelChart { kind: panel_kind::TABLE, ..Default::default() }, 1.0).unwrap();
            assert!(empty.rows.is_empty());
            assert_eq!(empty.height(), HEADER + 42.0);
        });
    }

    #[test]
    fn bar_palette_overrides_inherit_and_transparency_are_distinct() {
        let c = PanelChart { color: [240, 160, 48, 128], colors: vec![[0; 4], [212, 50, 130, 220], [1, 0, 0, 0]], ..Default::default() };
        assert_eq!(bar_tint(&c, 0), [240, 160, 48, 255]);
        assert_eq!(bar_tint(&c, 1), [212, 50, 130, 220]);
        assert_eq!(bar_tint(&c, 2), [1, 0, 0, 0]);
        assert_eq!(bar_tint(&c, 3), [240, 160, 48, 255]);
    }

    #[test]
    fn bar_bodies_preserve_tint_and_alpha() {
        for rgba in [[0, 158, 115, 255], [204, 121, 167, 255], [86, 180, 233, 128], [1, 0, 0, 0]] {
            let tint = c32(rgba, 1.0);
            assert_eq!(bar_body(tint), tint.gamma_multiply(0.72));
        }
        assert_eq!(bar_body(egui::Color32::TRANSPARENT), egui::Color32::TRANSPARENT);
    }

    #[test]
    fn contrasting_signals_keep_caller_edges_and_alpha_without_glow() {
        let ctx = egui::Context::default();
        let accent = c32([204, 121, 167, 128], 1.0);
        let points = vec![egui::pos2(10.0, 10.0), egui::pos2(40.0, 25.0)];
        let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
            panel_signal(ui.painter(), points.clone(), 2.0, accent, 0.0, draw_labels::GLASS_INK);
            panel_signal(ui.painter(), points.clone(), 2.0, egui::Color32::TRANSPARENT, 1.0, draw_labels::GLASS_INK);
        });
        let paths: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape {
            egui::Shape::Path(path) => Some(path), _ => None,
        }).collect();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].stroke, egui::Stroke::new(3.5, accent).into());
        assert_eq!(paths[1].stroke, egui::Stroke::new(2.0, faded(draw_labels::GLASS_INK, 128.0 / 255.0)).into());
        assert!(paths.iter().all(|path| path.points == points));
    }

    #[test]
    fn all_six_panel_families_paint_real_titles_and_data() {
        let ctx = egui::Context::default();
        for (kind, glass) in (0..=panel_kind::LEGEND).flat_map(|kind| [crate::draw::PanelGlass::White, crate::draw::PanelGlass::Black,
            crate::draw::PanelGlass::Color { light: false }, crate::draw::PanelGlass::Color { light: true }].map(|glass| (kind, glass))) {
            let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
                let data = PanelChart { kind, title: "Supplied title".into(), text: "Supplied caption".into(),
                    values: vec![-0.3, 0.2, 0.8], labels: vec!["A".into(), "B".into(), "C".into()],
                    rows: vec![("Source".into(), "Fixture".into())], color: visual::ACCENT, min: 0.0, max: 1.0,
                    map: (colormap::VIRIDIS, 0.0, 1.0), glow: 0.65, glass, ..Default::default() };
                chart(ui.painter(), egui::pos2(300.0, 300.0), &DrawPanel { key: ("test".into(), kind.to_string()),
                    chart: data, at: Some([0.5, 0.5]), opacity: 1.0 });
            });
            let texts: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()), _ => None,
            }).collect();
            assert!(texts.contains(&if kind == panel_kind::CHIP { "Supplied caption" } else { "Supplied title" }));
            assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(text)
                if text.override_text_color.is_none() && text.fallback_color == draw_labels::glass_ink(glass.light()))));
            assert!(output.shapes.iter().all(|s| !matches!(&s.shape, egui::Shape::Text(text)
                if text.override_text_color.is_some())));
            assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Path(path) if path.stroke.width > 0.0)));
            assert!(output.shapes.iter().all(|s| !matches!(&s.shape, egui::Shape::Path(path) if path.fill.a() > 0)));
            assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Mesh(mesh)
                if !mesh.vertices.is_empty() && mesh.vertices.iter().all(|v| (229..=240).contains(&v.color.a())))));
        }
    }

    #[test]
    fn transparent_panels_hide_text_frames_values_and_legends() {
        let ctx = egui::Context::default();
        let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            for (kind, glass) in (0..=panel_kind::LEGEND).flat_map(|kind| [crate::draw::PanelGlass::White, crate::draw::PanelGlass::Black,
                crate::draw::PanelGlass::Color { light: false }, crate::draw::PanelGlass::Color { light: true }].map(|glass| (kind, glass))) {
                let chart_data = PanelChart { kind, title: "Measured α".into(), text: "Visible only with alpha".into(), values: vec![0.4, 0.8], color: [90, 180, 220, 0], glass, ..Default::default() };
                let panel = DrawPanel { key: ("test".into(), kind.to_string()), chart: chart_data, at: Some([0.5, 0.5]), opacity: 1.0 };
                chart(painter, egui::pos2(150.0, 150.0), &panel);
                let content = PanelContent::prepare(painter, &panel.chart, 1.0);
                legend(painter, egui::Rect::from_min_size(egui::pos2(10.0, 10.0), content.size), &panel.chart, 1.0, &content);
            }
        });
        assert!(output.shapes.iter().all(|shape| matches!(shape.shape, egui::Shape::Noop)));
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
    skin: ControlSkin,
    props: gearbox_api::Props,
}

#[derive(Clone, Copy)]
struct ControlSkin {
    blur: f32,
    accent: egui::Color32,
    glass: PanelGlass,
    glow: f32,
}

impl ControlSkin {
    fn of(entity: &DrawEntity) -> Self {
        let props = entity.props();
        Self {
            accent: c32(if entity.color == 0 { visual::ACCENT } else { entity.color.to_le_bytes() }, 1.0),
            blur: visual::blur(entity, 1.0),
            glass: PanelGlass::of(&props.get("panel_finish").unwrap_or_default(), &props.get("panel_theme").unwrap_or_default()),
            glow: visual::effect(entity.glow, 0.65),
        }
    }

    fn ink(self) -> egui::Color32 { faded(draw_labels::glass_ink(self.glass.light()), self.accent.a() as f32 / 255.0) }
    fn muted(self) -> egui::Color32 { faded(draw_labels::glass_muted(self.glass.light()), self.accent.a() as f32 / 255.0) }

    fn frame(self, painter: &egui::Painter, rect: egui::Rect, selected: bool) {
        draw_labels::glass_frame(painter, rect, self.accent, selected, self.glow, self.glass, self.blur);
    }

    fn button(self, ui: &mut egui::Ui, rect: egui::Rect, text: &str, id: egui::Id) -> egui::Response {
        let response = ui.interact(rect, id, egui::Sense::click());
        self.frame(ui.painter(), rect, response.hovered() || response.has_focus());
        ui.painter().holographic_text(rect.center(), egui::Align2::CENTER_CENTER, text,
            draw_labels::font(ui.ctx(), 15.0, true), self.ink());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
        response
    }
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
    /// Data picks in flight: their request and the site the click saw.
    data_picks: HashMap<u64, crate::draw::pick::Pending>,

}

fn gizmo_color([r, g, b, _]: [u8; 4]) -> GizmoColor { GizmoColor::from_rgb(r, g, b) }

fn handle_dragging(active: bool, focused: bool, pressed: bool, down: bool, over: bool) -> bool {
    down && (active || (pressed && over && focused))
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
    camera(world).map(|(_, gt, fov, near, site, home)| (gt, fov, near, site, home))
}

fn camera(world: &mut World) -> Option<(Entity, GlobalTransform, f32, f32, GlobalTransform, crate::globe::SiteEntry)> {
    let mut q = world.query_filtered::<(Entity, &GlobalTransform, &Projection), With<mara_bevy::ChaseCamera>>();
    let (entity, gt, projection) = q.iter(world).next().map(|(e, g, p)| (e, *g, p.clone()))?;
    let (fov, near) = match projection {
        Projection::Perspective(p) => (p.fov, p.near),
        _ => (0.8, 0.1),
    };
    let home = world.get_resource::<crate::globe::Sites>()?.home().clone();
    let site = *world.get::<GlobalTransform>(home.entity)?;
    Some((entity, gt, fov, near, site, home))
}

/// Entity-frame coordinates of the frame into the home site's.
fn frame_to_site(frame: &DrawFrame, home: &crate::globe::SiteEntry) -> Transform {
    site_transform(frame, Transform::IDENTITY, home).unwrap_or_else(enu_to_site)
}

/// The cursor in normalized device coordinates of the viewport, y up.
fn ndc(cursor: egui::Pos2, viewport: egui::Rect) -> Vec2 {
    Vec2::new((cursor.x - viewport.min.x) / viewport.width() * 2.0 - 1.0, 1.0 - (cursor.y - viewport.min.y) / viewport.height() * 2.0)
}

/// Where the ray through `cursor` meets the ground, in the home site's frame.
fn ground_hit(cursor: egui::Pos2, viewport: egui::Rect, eye: &GlobalTransform, fov: f32, site: &GlobalTransform) -> Option<Vec3> {
    let ndc = ndc(cursor, viewport);
    let aspect = viewport.width() / viewport.height().max(1.0);
    let t = (0.5 * fov).tan();
    let dir_view = Vec3::new(ndc.x * t * aspect, ndc.y * t, -1.0).normalize();
    let (_, rot, origin) = eye.to_scale_rotation_translation();
    let inv = site.affine().inverse();
    let o = inv.transform_point3(origin);
    let d = inv.transform_vector3(rot * dir_view).normalize();
    crate::draw::pick::terrain_distance(o, d).map(|t| o + d * t)
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
    /// Answers the data picks the renderer resolved and removes their requests.
    fn deliver_picks(&mut self, world: &mut World) {
        let Some(picks) = world.get_resource::<crate::draw::pick::Picks>().cloned() else {
            return;
        };
        for (id, found) in picks.take_owned(|id| self.data_picks.contains_key(&id)) {
            if let Some(pending) = self.data_picks.remove(&id) {
                crate::draw::pick::publish(world, &pending, found);
            }
        }
    }

    pub(super) fn needs_frost(&self) -> bool {
        self.requests.iter().any(|r| r.skin.accent.a() > 0 && (r.kind != interact_kind::HANDLE
            || self.menu.as_ref().is_some_and(|(ns, id, _)| r.ns == *ns && r.id == *id)))
    }

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
                        skin: ControlSkin::of(&item.entity),
                        props: item.entity.props(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let Some(viewport) = self.viewport else {
            return;
        };
        let Some((camera, eye, fov, near, site, home)) = camera(world) else {
            return;
        };
        let (pos, pressed, released, down, secondary) = egui.input(|i| {
            (i.pointer.latest_pos(), i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.primary_down(), i.pointer.secondary_pressed())
        });
        let over = self.menu.is_none() && pos.is_some_and(|p| viewport.contains(p)) && !egui.is_pointer_over_egui();
        let mut grab = self.menu.is_some();
        self.hover = None;
        self.deliver_picks(world);
        let pointer_pick = |r: &&Request| r.kind == interact_kind::PICK_DATA && r.props.get("ndc").is_none();
        if let Some(r) = self.requests.iter().find(pointer_pick).cloned() {
            let picks = world.get_resource::<crate::draw::pick::Picks>().cloned();
            let queued = self.data_picks.iter().find(|(_, q)| q.ns == r.ns && q.id == r.id).map(|(id, _)| *id);
            if egui.input(|i| i.key_pressed(egui::Key::Escape)) {
                if let Some(id) = queued {
                    self.data_picks.remove(&id);
                    if let Some(picks) = &picks {
                        picks.cancel(id);
                    }
                }
                answer(world, &r, "cancel", Vec3::ZERO, &[]);
                remove(world, &r);
                return;
            }
            if over && queued.is_none() {
                grab = true;
                self.hover = pos;
                if pressed && let (Some(p), Some(picks)) = (pos, picks) {
                    let id = picks.next_id();
                    let at = ndc(p, viewport);
                    picks.submit(crate::draw::pick::Pick {
                        id,
                        camera,
                        eye,
                        fov,
                        aspect: viewport.width() / viewport.height().max(1.0),
                        ndc: at,
                        prefix: r.props.get("prefix").unwrap_or_default(),
                        occluder: f32::INFINITY,
                    });
                    let inspect = r.props.get("inspect").is_some();
                    let pending = crate::draw::pick::Pending { ns: r.ns.clone(), id: r.id.clone(), frame: r.frame.clone(), site, home: home.clone(), ndc: at, inspect };
                    self.data_picks.insert(id, pending);
                }
            }
        }
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
            let (gizmo, was_dragging) = self.gizmos.entry(key.clone()).or_insert_with(|| (GizmoCore::default(), false));
            gizmo.update_config(GizmoConfig {
                view_matrix: rows(Mat4::from(eye.affine().inverse())),
                projection_matrix: rows(clip_from_view),
                viewport: GizmoRect::from_x_y_ranges(viewport.min.x..=viewport.max.x, viewport.min.y..=viewport.max.y),
                modes,
                orientation: GizmoOrientation::Local,
                visuals: GizmoVisuals {
                    gizmo_size: (75.0 * r.scale.min(2.0)).clamp(40.0, 150.0),
                    x_color: gizmo_color(visual::AXIS_COLORS[0]),
                    y_color: gizmo_color(visual::AXIS_COLORS[1]),
                    z_color: gizmo_color(visual::AXIS_COLORS[2]),
                    inactive_alpha: 0.8 * r.skin.accent.a() as f32 / 255.0,
                    highlight_alpha: r.skin.accent.a() as f32 / 255.0,
                    highlight_color: Some(gizmo_color(visual::INK)),
                    ..Default::default()
                },
                pixels_per_point: egui.pixels_per_point(),
                ..Default::default()
            });
            let interaction = GizmoInteraction {
                cursor_pos: pos.map(|p| (p.x, p.y)).unwrap_or((-1.0, -1.0)),
                hovered: over,
                drag_started: pressed && over,
                dragging: down && self.menu.is_none(),
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
            let dragging = handle_dragging(*was_dragging, focused, pressed, down, over);
            grab |= focused;
            let entity_pose = |wp: Transform| -> Transform {
                let local = GlobalTransform::from(to_site).affine().inverse() * site.affine().inverse() * wp.compute_affine();
                Transform::from_matrix(Mat4::from(local))
            };
            if dragging && !*was_dragging {
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
            if !dragging && *was_dragging {
                answer(world, r, "mouse_up", r.pose.translation, &[]);
            }
            *was_dragging = dragging;
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
        let requests = self.requests.clone();
        let mut steps = 0;
        for r in &requests {
            let skin = r.skin;
            let accent = skin.accent;
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
                            let label = if full { "Continue" } else { "Next" };
                            let title = ui.painter().layout_no_wrap(caption.clone(), draw_labels::font(egui, 16.0, true), skin.ink());
                            let key = ui.painter().layout_no_wrap("[N]".into(), draw_labels::font(egui, 12.0, false), skin.muted());
                            let width = title.size().x + key.size().x + 130.0;
                            let (plate, _) = ui.allocate_exact_size(egui::vec2(width, 44.0), egui::Sense::hover());
                            skin.frame(ui.painter(), plate, false);
                            ui.painter().galley(egui::pos2(plate.left() + 14.0, plate.center().y - title.size().y * 0.5), title.clone(), skin.ink());
                            ui.painter().galley(egui::pos2(plate.left() + 24.0 + title.size().x, plate.center().y - key.size().y * 0.5), key, skin.muted());
                            let button = egui::Rect::from_min_max(egui::pos2(plate.right() - 82.0, plate.top() + 6.0), plate.right_bottom() - egui::vec2(6.0, 6.0));
                            clicked = skin.button(ui, button, label, ui.id().with("advance")).clicked();
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
                    let galley = painter.layout_no_wrap(format!("{text}   ·   Esc cancels"), draw_labels::font(painter.ctx(), 14.0, false), skin.ink());
                    let plate = egui::Rect::from_center_size(at, galley.size() + egui::vec2(28.0, 14.0));
                    skin.frame(&painter, plate, false);
                    painter.galley(plate.center() - 0.5 * galley.size(), galley, skin.ink());
                    if let Some(p) = self.hover {
                        draw_labels::corners(&painter, egui::Rect::from_center_size(p, egui::vec2(22.0, 22.0)), accent, true);
                        draw_labels::anchor(&painter, p, accent);
                        egui.request_repaint();
                    }
                    if let (Some((from, _)), Some(to)) = (self.pressed, self.hover) {
                        let d = to - from;
                        if d.length() > 4.0 {
                            let n = d.normalized();
                            let side = egui::vec2(-n.y, n.x);
                            draw_labels::lit_path(&painter, vec![from, to], 1.5 * visual::RULE_PX, accent, skin.glow);
                            draw_labels::lit_path(&painter, vec![to - n * 9.0 + side * 6.0, to, to - n * 9.0 - side * 6.0],
                                1.5 * visual::RULE_PX, accent, skin.glow);
                        }
                        draw_labels::anchor(&painter, from, accent);
                    }
                }
                _ => {}
            }
        }
        let gizmo_painter = egui.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("gearbox_draw_handles"))).with_clip_rect(rect);
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
            gizmo_painter.add(egui::Shape::mesh(mesh));
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
                let width = entries.iter().map(|text| ui.painter().layout_no_wrap(text.clone(),
                    draw_labels::font(egui, 15.0, true), r.skin.ink()).size().x + 28.0).fold(128.0_f32, f32::max);
                for (i, e) in entries.iter().enumerate() {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 32.0), egui::Sense::hover());
                    if r.skin.button(ui, rect, e, ui.id().with(i)).clicked() {
                        picked = Some(e.clone());
                    }
                }
                close = ui.input(|i| i.key_pressed(egui::Key::Escape));
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
