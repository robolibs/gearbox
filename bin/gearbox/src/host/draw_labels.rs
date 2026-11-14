use std::sync::Arc;

use crate::draw::{DrawLabel, PanelGlass, visual};

const IDENTITY: &str = "gearbox_world_identity";
const BODY: &str = "gearbox_world_body";
pub(super) const GLASS_INK: egui::Color32 = egui::Color32::from_rgb(246, 248, 250);
pub(super) const GLASS_MUTED: egui::Color32 = egui::Color32::from_rgb(222, 228, 234);

pub(super) fn glass_ink(light: bool) -> egui::Color32 {
    if light { egui::Color32::from_rgb(15, 21, 25) } else { GLASS_INK }
}

pub(super) fn glass_muted(light: bool) -> egui::Color32 {
    if light { egui::Color32::from_rgb(25, 31, 36) } else { GLASS_MUTED }
}

pub(super) fn font(ctx: &egui::Context, size: f32, strong: bool) -> egui::FontId {
    let family = egui::FontFamily::Name(if strong { IDENTITY } else { BODY }.into());
    if ctx.fonts(|fonts| fonts.definitions().families.contains_key(&family)) {
        return egui::FontId::new(size, family);
    }
    let mut definitions = ctx.fonts(|fonts| fonts.definitions().clone());
    let fallback = definitions.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, bytes) in [
        (IDENTITY, include_bytes!("../../assets/fonts/Rajdhani-SemiBold.ttf").as_slice()),
        (BODY, include_bytes!("../../assets/fonts/Rajdhani-Medium.ttf").as_slice()),
    ] {
        definitions.font_data.insert(name.into(), egui::FontData::from_static(bytes).into());
        definitions.families.insert(egui::FontFamily::Name(name.into()), std::iter::once(name.to_owned()).chain(fallback.iter().cloned()).collect());
    }
    ctx.set_fonts(definitions);
    egui::FontId::proportional(size)
}

pub(super) fn color(rgba: [u8; 4], opacity: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], (rgba[3] as f32 * opacity.clamp(0.0, 1.0)) as u8)
}

fn corner_segments(rect: egui::Rect, length: f32) -> [[egui::Pos2; 2]; 8] {
    let length = length.min(rect.width() * 0.25).min(rect.height() * 0.4).max(0.0);
    let mut segments = [[egui::Pos2::ZERO; 2]; 8];
    for (i, (at, direction)) in [
        (rect.left_top(), egui::vec2(1.0, 1.0)), (rect.right_top(), egui::vec2(-1.0, 1.0)),
        (rect.right_bottom(), egui::vec2(-1.0, -1.0)), (rect.left_bottom(), egui::vec2(1.0, -1.0)),
    ].into_iter().enumerate() {
        segments[i * 2] = [at, at + egui::vec2(direction.x * length, 0.0)];
        segments[i * 2 + 1] = [at, at + egui::vec2(0.0, direction.y * length)];
    }
    segments
}

pub(super) fn corners(painter: &egui::Painter, rect: egui::Rect, accent: egui::Color32, selected: bool) {
    for segment in corner_segments(rect, visual::CORNER_PX) {
        painter.line_segment(segment, egui::Stroke::new(visual::RULE_PX * visual::STRUCTURE_CORNER_GAIN, accent.gamma_multiply(if selected { 1.0 } else { 0.8 })));
    }
}

pub(super) fn lit_path(painter: &egui::Painter, points: Vec<egui::Pos2>, width: f32, ink: egui::Color32, glow: f32) {
    if ink.a() == 0 || points.len() < 2 { return; }
    let glow = glow.clamp(0.0, 2.0);
    if glow > 0.0 {
        for (spread, alpha) in [(4.0, 0.055), (2.0, 0.14)] {
            painter.add(egui::Shape::line(points.clone(), egui::Stroke::new(width + spread, ink.gamma_multiply(alpha * glow))));
        }
    }
    painter.add(egui::Shape::line(points, egui::Stroke::new(width, ink)));
}

fn frame_polygon(rect: egui::Rect) -> Vec<egui::Pos2> {
    let cut = 7.0_f32.min(rect.height() * 0.2);
    vec![rect.left_top(), rect.right_top() - egui::vec2(cut, 0.0), rect.right_top() + egui::vec2(0.0, cut),
        rect.right_bottom(), rect.left_bottom() + egui::vec2(cut, 0.0), rect.left_bottom() - egui::vec2(0.0, cut)]
}

fn glass_tint(accent: egui::Color32, coverage: f32, light: bool) -> egui::Color32 {
    let [r, g, b, a] = accent.to_srgba_unmultiplied();
    let shade = |v: u8| if light { (165.0 + v as f32 * 0.16) as u8 } else { (16.0 + v as f32 * 0.14) as u8 };
    egui::Color32::from_rgba_unmultiplied(shade(r), shade(g), shade(b), (a as f32 * coverage) as u8)
}

fn frost_tint(accent: egui::Color32, coverage: f32, glass: PanelGlass) -> egui::Color32 {
    let [r, g, b, a] = accent.to_srgba_unmultiplied();
    let shade = |v: u8| match glass {
        PanelGlass::Color { light: true } => (76.0 + v as f32 * 0.70) as u8,
        PanelGlass::Color { light: false } => (v as f32 * 0.60) as u8,
        PanelGlass::White => (188.0 + v as f32 * 0.24) as u8,
        PanelGlass::Black => (12.0 + v as f32 * 0.22) as u8,
    };
    egui::Color32::from_rgba_unmultiplied(shade(r), shade(g), shade(b), (a as f32 * coverage) as u8)
}

fn film_coverage(frosted: bool, glass: PanelGlass) -> [f32; 3] {
    match (frosted, glass) {
        (false, _) => [0.90, 0.94, 0.92],
        (true, PanelGlass::White) => [0.50, 0.58, 0.54],
        (true, _) => [0.18, 0.26, 0.22],
    }
}

pub(super) fn glass_frame(painter: &egui::Painter, rect: egui::Rect, accent: egui::Color32, selected: bool, glow: f32, glass: PanelGlass, blur: f32) {
    if accent.a() == 0 { return; }
    let points = frame_polygon(rect);
    let frosted = super::draw_frost::paint(painter, &points, accent.a() as f32 / 255.0, glass.light(), blur);
    let tint = |coverage| if frosted { frost_tint(accent, coverage, glass) } else { glass_tint(accent, coverage, glass.light()) };
    let [center, top, bottom] = film_coverage(frosted, glass);
    let mut film = egui::Mesh::default();
    film.colored_vertex(rect.center(), tint(center));
    for point in &points {
        let height = ((point.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        film.colored_vertex(*point, tint(top + (bottom - top) * height));
    }
    for i in 0..points.len() { film.add_triangle(0, i as u32 + 1, ((i + 1) % points.len()) as u32 + 1); }
    painter.add(egui::Shape::mesh(film));
    lit_frame(painter, rect, egui::Color32::TRANSPARENT, accent, selected, glow);
}

pub(super) fn lit_frame(painter: &egui::Painter, rect: egui::Rect, background: egui::Color32, accent: egui::Color32, selected: bool, glow: f32) {
    let points = frame_polygon(rect);
    painter.add(egui::Shape::convex_polygon(points.clone(), background, egui::Stroke::NONE));
    for i in 0..points.len() {
        let (a, b) = (points[i], points[(i + 1) % points.len()]);
        let fraction = (visual::CORNER_PX / a.distance(b).max(1.0)).min(gearbox_api::wire::draw_widgets::BRACKET_FRACTION);
        let (start, end) = (a.lerp(b, fraction), b.lerp(a, fraction));
        lit_path(painter, vec![start, end], visual::RULE_PX, accent.gamma_multiply(if selected { 0.8 } else { 0.45 }), glow);
        for arm in [vec![a, start], vec![end, b]] {
            lit_path(painter, arm, visual::RULE_PX * visual::STRUCTURE_CORNER_GAIN, accent, glow);
        }
    }
}

pub(super) fn anchor(painter: &egui::Painter, at: egui::Pos2, accent: egui::Color32) {
    painter.rect_stroke(egui::Rect::from_center_size(at, egui::vec2(5.0, 5.0)), 0.0, egui::Stroke::new(visual::RULE_PX, accent), egui::StrokeKind::Middle);
}

pub(super) fn leader(painter: &egui::Painter, at: egui::Pos2, bounds: egui::Rect, accent: egui::Color32, glow: f32, occupied: &[egui::Rect]) {
    for path in super::draw_layout::visible_leader(at, bounds, occupied) {
        lit_path(painter, path, visual::RULE_PX, accent.gamma_multiply(0.8), glow);
    }
    if !bounds.contains(at) && occupied.iter().all(|r| !r.expand(4.0).contains(at)) { anchor(painter, at, accent); }
}

pub(super) fn icon(painter: &egui::Painter, at: egui::Pos2, size: f32, kind: &str, accent: egui::Color32) -> bool {
    let r = size * 0.5;
    let stroke = egui::Stroke::new(visual::RULE_PX, accent);
    let segment = |a: [f32; 2], b: [f32; 2]| { painter.line_segment([at + egui::vec2(a[0], a[1]) * r, at + egui::vec2(b[0], b[1]) * r], stroke); };
    match kind {
        "robot" => {
            painter.rect_stroke(egui::Rect::from_center_size(at, egui::vec2(size * 0.65, size * 0.6)), 0.0, stroke, egui::StrokeKind::Middle);
            for x in [-0.65, 0.65] { segment([x, -0.7], [x, 0.7]); }
            segment([0.0, -0.3], [0.0, -0.9]);
        }
        "sensor" => {
            painter.circle_stroke(at, r * 0.65, stroke);
            segment([-1.0, 0.0], [-0.4, 0.0]); segment([0.4, 0.0], [1.0, 0.0]); segment([0.0, -1.0], [0.0, -0.4]);
        }
        "destination" => {
            for (a, b) in [([0.0, -1.0], [0.8, 0.0]), ([0.8, 0.0], [0.0, 1.0]), ([0.0, 1.0], [-0.8, 0.0]), ([-0.8, 0.0], [0.0, -1.0])] { segment(a, b); }
            painter.rect_filled(egui::Rect::from_center_size(at, egui::vec2(2.0, 2.0)), 0.0, accent);
        }
        "measurement" => {
            segment([-0.9, 0.0], [0.9, 0.0]);
            for x in [-0.9, 0.0, 0.9] { segment([x, -0.5], [x, 0.5]); }
        }
        "obstacle" => {
            for (a, b) in [([-0.9, 0.75], [0.0, -0.9]), ([0.0, -0.9], [0.9, 0.75]), ([0.9, 0.75], [-0.9, 0.75])] { segment(a, b); }
            segment([0.0, -0.25], [0.0, 0.3]);
        }
        "frame" => {
            segment([-0.5, 0.5], [-0.5, -0.9]); segment([-0.5, 0.5], [0.9, 0.5]); segment([-0.5, 0.5], [0.5, -0.5]);
        }
        _ => return false,
    }
    true
}

fn text(painter: &egui::Painter, value: &str, size: f32, strong: bool, ink: egui::Color32, width: f32, tracking: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    job.append(value, 0.0, egui::TextFormat { font_id: font(painter.ctx(), size, strong), extra_letter_spacing: tracking, color: ink, ..Default::default() });
    painter.layout_job(job)
}

fn text_edge_color(ink: egui::Color32, source: [u8; 4]) -> egui::Color32 {
    if ink.a() == 0 { return egui::Color32::TRANSPARENT; }
    let luminance = |c: egui::Rgba| 0.2126 * c.r() + 0.7152 * c.g() + 0.0722 * c.b();
    let foreground = luminance(egui::Rgba::from(egui::Color32::from_rgb(source[0], source[1], source[2])));
    let light = luminance(egui::Rgba::from(color(visual::INK, 1.0)));
    let alpha = (ink.a() as f32 * (175.0 / 255.0)) as u8;
    if (light + 0.05) / (foreground + 0.05) > (foreground + 0.05) / 0.05 {
        egui::Color32::from_rgba_unmultiplied(visual::INK[0], visual::INK[1], visual::INK[2], alpha)
    } else {
        egui::Color32::from_black_alpha(alpha)
    }
}

fn edged_text(painter: &egui::Painter, at: egui::Pos2, galley: Arc<egui::Galley>, ink: egui::Color32, source: [u8; 4]) {
    let key = text_edge_color(ink, source);
    let small = galley.job.sections.first().is_some_and(|section| section.format.font_id.size < 14.0);
    let edge = if small { 0.5 } else { 0.75 };
    for offset in [egui::vec2(-edge, 0.0), egui::vec2(edge, 0.0), egui::vec2(0.0, -edge), egui::vec2(0.0, edge)] {
        painter.galley_with_override_text_color(at + offset, galley.clone(), key);
    }
    painter.galley(at, galley, ink);
}

fn place(bounds: egui::Rect, viewport: egui::Rect, occupied: &[egui::Rect], movable: bool) -> egui::Rect {
    if movable { super::draw_layout::place(bounds, viewport, occupied, 8.0) } else { bounds }
}

struct LabelLayout<'a> {
    label: &'a DrawLabel,
    at: egui::Pos2,
    opacity: f32,
    ink: egui::Color32,
    accent: egui::Color32,
    size: f32,
    identity: Arc<egui::Galley>,
    category: Option<Arc<egui::Galley>>,
    detail: Option<Arc<egui::Galley>>,
    icon_width: f32,
    category_height: f32,
    pad: egui::Vec2,
    initial: egui::Rect,
    bounds: Option<egui::Rect>,
    hidden: bool,
}

pub(super) struct PreparedLabels<'a> {
    layouts: Vec<LabelLayout<'a>>,
    pub occupied: Vec<egui::Rect>,
}

#[cfg(test)]
pub(super) fn paint(ctx: &egui::Context, viewport: egui::Rect, labels: &[DrawLabel]) {
    let mut prepared = prepare(ctx, viewport, labels);
    place_remaining(viewport, &mut prepared);
    paint_prepared(ctx, viewport, prepared);
}

#[cfg(test)]
pub(super) fn prepare<'a>(ctx: &egui::Context, viewport: egui::Rect, labels: &'a [DrawLabel]) -> PreparedLabels<'a> {
    prepare_reserved(ctx, viewport, labels, &[])
}

pub(super) fn prepare_reserved<'a>(ctx: &egui::Context, viewport: egui::Rect, labels: &'a [DrawLabel], reserved: &[egui::Rect]) -> PreparedLabels<'a> {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("gearbox_draw_labels"))).with_clip_rect(viewport);
    let mut prepared = PreparedLabels { layouts: Vec::new(), occupied: reserved.to_vec() };
    for label in labels {
        if !label.opacity.is_finite() || label.opacity <= 0.0 || label.color[3] == 0 || label.text.is_empty() || !label.size_px.is_finite() || label.at.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) { continue; }
        let at = viewport.min + egui::vec2(label.at[0] * viewport.width(), label.at[1] * viewport.height());
        let opacity = label.opacity * label.color[3] as f32 / 255.0;
        let ink = color(label.color, label.opacity);
        let accent = color(label.accent, label.opacity);
        let size = label.size_px.max(5.0);
        let width = (viewport.width() * 0.35).clamp(120.0, 340.0).max(size * 4.0);
        let identity = text(&painter, &label.text, size, true, ink, width, 0.0);
        let meta = &label.annotation;
        let category = (!meta.category.is_empty() && size >= 11.0).then(|| text(&painter, &meta.category, (size * 0.72).max(11.0), false, accent, width, 0.8));
        let detail = (!meta.detail.is_empty()).then(|| text(&painter, &meta.detail, (size * 0.83).max(11.0), false, color(visual::SECONDARY, opacity), width, 0.0));
        let icon_width = if matches!(meta.icon.as_str(), "robot" | "sensor" | "destination" | "measurement" | "obstacle" | "frame") && size >= 11.0 { size + 7.0 } else { 0.0 };
        let gap = visual::SPACE_PX;
        let category_height = category.as_ref().map_or(0.0, |g| g.size().y + 1.0);
        let detail_height = detail.as_ref().map_or(0.0, |g| g.size().y + gap + 2.0);
        let content = egui::vec2(identity.size().x.max(category.as_ref().map_or(0.0, |g| g.size().x)).max(detail.as_ref().map_or(0.0, |g| g.size().x)) + icon_width, identity.size().y + category_height + detail_height);
        let pad = if meta.selected { egui::vec2(8.0, 5.0) } else { egui::vec2(3.0, 2.0) };
        let origin = if label.centred { at - content * 0.5 } else { at + egui::vec2(20.0, -content.y - 12.0) };
        let initial = egui::Rect::from_min_size(origin, content).expand2(pad);
        prepared.layouts.push(LabelLayout { label, at, opacity, ink, accent, size, identity, category, detail,
            icon_width, category_height, pad, initial, bounds: None, hidden: false });
    }
    let group = |l: &LabelLayout| if l.label.centred && !l.label.annotation.selected { 0 } else if l.label.annotation.selected { 1 } else { 2 };
    prepared.layouts.sort_by(|a, b| group(a).cmp(&group(b)).then(b.size.total_cmp(&a.size)));
    let mut fixed: Vec<egui::Rect> = Vec::new();
    for layout in &mut prepared.layouts {
        if !layout.label.centred && !layout.label.annotation.selected { continue; }
        if group(layout) == 0 {
            // Anchored labels never move; a nearer (larger) one hides any it would overprint.
            if fixed.iter().any(|other| other.intersects(layout.initial.shrink(1.0))) {
                layout.hidden = true;
                continue;
            }
            fixed.push(layout.initial);
        }
        let bounds = place(layout.initial, viewport, &prepared.occupied, layout.label.annotation.selected);
        layout.bounds = Some(bounds);
        prepared.occupied.push(bounds);
    }
    prepared
}

pub(super) fn place_remaining(viewport: egui::Rect, prepared: &mut PreparedLabels<'_>) {
    for layout in &mut prepared.layouts {
        if layout.bounds.is_some() || layout.hidden { continue; }
        let bounds = place(layout.initial, viewport, &prepared.occupied, true);
        prepared.occupied.push(bounds);
        layout.bounds = Some(bounds);
    }
}

pub(super) fn paint_prepared(ctx: &egui::Context, viewport: egui::Rect, prepared: PreparedLabels<'_>) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("gearbox_draw_labels"))).with_clip_rect(viewport);
    for layout in prepared.layouts.into_iter().filter(|layout| !layout.hidden) {
        let LabelLayout { label, at, opacity, ink, accent, size, identity, category, detail, icon_width, category_height, pad, initial, bounds, .. } = layout;
        let bounds = bounds.unwrap_or(initial);
        let meta = &label.annotation;
        let gap = visual::SPACE_PX;
        let origin = bounds.min + pad;
        let backed = (meta.selected && !meta.background_explicit) || label.background[3] > 0;
        let paint_text = |at, galley: Arc<egui::Galley>, ink, source| {
            if backed { painter.galley(at, galley, ink); }
            else { edged_text(&painter, at, galley, ink, source); }
        };
        if label.background[3] > 0 && let Some(blur) = meta.blur {
            super::draw_frost::paint(&painter, &frame_polygon(bounds), opacity * label.background[3] as f32 / 255.0, true, blur);
        }
        if meta.selected {
            let background = if meta.background_explicit || label.background[3] > 0 { label.background } else { visual::BACKGROUND };
            if meta.background_explicit || background[3] > 0 {
                lit_frame(&painter, bounds, color(background, opacity * 0.9), accent, true, meta.glow.unwrap_or(0.65));
            } else {
                glass_frame(&painter, bounds, accent, true, meta.glow.unwrap_or(0.65), PanelGlass::Black, meta.blur.unwrap_or(1.0));
            }
        } else if label.background[3] > 0 {
            painter.rect_filled(bounds, 0.0, color(label.background, opacity));
        }
        if !label.centred || bounds != initial {
            leader(&painter, at, bounds, accent, meta.glow.unwrap_or(0.65), &prepared.occupied);
        }
        let identity_at = origin + egui::vec2(icon_width, category_height);
        if icon_width > 0.0 { icon(&painter, origin + egui::vec2(size * 0.5, category_height + identity.size().y * 0.5), size * 0.8, &meta.icon, accent); }
        if let Some(category) = category { paint_text(origin + egui::vec2(icon_width, 0.0), category, accent, label.accent); }
        paint_text(identity_at, identity.clone(), ink, label.color);
        if let Some(detail) = detail {
            let y = identity_at.y + identity.size().y + gap * 0.5;
            painter.line_segment([egui::pos2(identity_at.x, y), egui::pos2(bounds.right() - pad.x, y)], egui::Stroke::new(visual::RULE_PX, accent.gamma_multiply(0.45)));
            paint_text(egui::pos2(identity_at.x, y + gap), detail, color(visual::SECONDARY, opacity), visual::SECONDARY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frosted_film_keeps_caller_hue_and_low_coverage_in_both_themes() {
        for light in [false, true] {
            for channel in 0..3 {
                let mut rgb = [30; 3]; rgb[channel] = 240;
                let accent = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                for coverage in [0.18, 0.22, 0.26] {
                    let tint = frost_tint(accent, coverage, PanelGlass::Color { light }).to_srgba_unmultiplied();
                    assert!((45..=67).contains(&tint[3]));
                    assert!((0..3).filter(|i| *i != channel).all(|i| tint[channel] > tint[i] + 100));
                }
            }
            assert_eq!(frost_tint(egui::Color32::TRANSPARENT, 0.26, PanelGlass::Color { light }), egui::Color32::TRANSPARENT);
        }
    }

    #[test]
    fn neutral_film_colors_keep_caller_hue_at_the_given_coverage() {
        for (glass, light) in [(PanelGlass::White, true), (PanelGlass::Black, false)] {
            for channel in 0..3 {
                let mut rgb = [30; 3]; rgb[channel] = 240;
                for alpha in [0, 128, 255] {
                    let accent = egui::Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], alpha);
                    for coverage in film_coverage(true, glass) {
                        let neutral = frost_tint(accent, coverage, glass).to_srgba_unmultiplied();
                        let colored = frost_tint(accent, coverage, PanelGlass::Color { light }).to_srgba_unmultiplied();
                        assert_eq!(neutral[3], colored[3]);
                        if alpha == 0 { assert_eq!(neutral, [0; 4]); continue; }
                        let other = (channel + 1) % 3;
                        assert!(neutral[channel] > neutral[other]);
                        assert!(neutral[channel] - neutral[other] < (colored[channel] - colored[other]) / 2);
                        assert!(neutral[..3].iter().all(|v| if light { *v > 180 && *v < 252 } else { *v > 5 && *v < 80 }));
                    }
                }
            }
        }
    }

    #[test]
    fn white_film_is_visibly_milkier_without_changing_other_finishes() {
        let accent = egui::Color32::from_rgb(207, 114, 168);
        for coverage in film_coverage(true, PanelGlass::White) {
            assert!((0.50..=0.58).contains(&coverage));
            let film = frost_tint(accent, coverage, PanelGlass::White).to_srgba_unmultiplied();
            let old = frost_tint(accent, 0.26, PanelGlass::White).to_srgba_unmultiplied();
            for channel in 0..3 {
                let composite = |rgba: [u8; 4]| 80.0 + (rgba[channel] as f32 - 80.0) * rgba[3] as f32 / 255.0;
                assert!(composite(film) > composite(old) + 28.0);
            }
            assert!(film[3] < 150);
            assert_eq!(frost_tint(egui::Color32::TRANSPARENT, coverage, PanelGlass::White), egui::Color32::TRANSPARENT);
        }
        for glass in [PanelGlass::Black, PanelGlass::Color { light: true }, PanelGlass::Color { light: false }] {
            assert_eq!(film_coverage(true, glass), [0.18, 0.26, 0.22]);
        }
    }

    #[test]
    fn crowded_frame_names_keep_separate_labels_inside_the_viewport() {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0));
        for at in [egui::pos2(440.0, 340.0), egui::pos2(965.0, 690.0)] {
            let initial = egui::Rect::from_min_size(at, egui::vec2(120.0, 20.0));
            let mut occupied = Vec::new();
            for _ in 0..12 {
                let placed = place(initial, viewport, &occupied, true);
                assert!(viewport.shrink(5.0).contains_rect(placed));
                assert!(occupied.iter().all(|r| !placed.expand(3.0).intersects(*r)));
                occupied.push(placed);
            }
            assert_eq!(place(initial, viewport, &occupied, false), initial);
        }
    }

    #[test]
    fn fallback_glass_ink_contrasts_with_black_and_white_worlds() {
        let linear = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        let luminance = |rgb: [f32; 3]| rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        for (rgb, light) in [[0, 0, 0], [255, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0], [204, 121, 167]]
            .into_iter().flat_map(|rgb| [false, true].map(|light| (rgb, light))) {
            let accent = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            for coverage in [0.90, 0.92, 0.94] {
                let film = glass_tint(accent, coverage, light).to_srgba_unmultiplied();
                assert!(film[3] < 255);
                let a = film[3] as f32 / 255.0;
                for world in [0.0, 1.0] {
                    let bg_linear = luminance(std::array::from_fn(|i| linear(film[i] as f32 / 255.0) * a + world * (1.0 - a)));
                    let bg_gamma = luminance(std::array::from_fn(|i| linear(film[i] as f32 / 255.0 * a + world * (1.0 - a))));
                    for ink in [glass_ink(light), glass_muted(light)] {
                        let channels = ink.to_srgba_unmultiplied();
                        let foreground = luminance(std::array::from_fn(|i| linear(channels[i] as f32 / 255.0)));
                        for background in [bg_linear, bg_gamma] {
                            let contrast = (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05);
                            assert!(contrast >= 4.5, "{rgb:?} light {light}, coverage {coverage}, world {world}: {contrast}");
                        }
                    }
                }
            }
        }
        for light in [false, true] {
            assert_eq!(glass_tint(egui::Color32::TRANSPARENT, 0.94, light), egui::Color32::TRANSPARENT);
        }
    }

    #[test]
    fn panel_frame_accents_follow_every_actual_vertex_with_shared_weight() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(180.0, 90.0));
        let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
            lit_frame(ui.painter(), rect, egui::Color32::TRANSPARENT, egui::Color32::WHITE, false, 0.0);
        });
        let paths: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape {
            egui::Shape::Path(path) if path.stroke.width > visual::RULE_PX => Some(path), _ => None,
        }).collect();
        assert_eq!(paths.len(), 12);
        for point in frame_polygon(rect) {
            assert_eq!(paths.iter().filter(|path| path.points.contains(&point)).count(), 2);
        }
        for path in paths {
            assert_eq!(path.stroke.width, visual::RULE_PX * visual::STRUCTURE_CORNER_GAIN);
            assert!(path.points[0].distance(path.points[1]) <= visual::CORNER_PX + 0.001);
        }
    }

    #[test]
    fn corners_are_short_and_leave_open_edges() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(180.0, 30.0));
        for segment in corner_segments(rect, 6.0) {
            assert!((segment[0].distance(segment[1]) - 6.0).abs() < 1e-5);
            assert!(segment.iter().all(|p| rect.contains(*p)));
        }
    }

    #[test]
    fn placement_is_stable_bounded_and_avoids_neighbors() {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let original = egui::Rect::from_min_size(egui::pos2(100.0, 120.0), egui::vec2(100.0, 30.0));
        let placed = place(original, viewport, &[original], true);
        assert!(!placed.intersects(original));
        assert!(viewport.contains_rect(placed));
        assert_eq!(placed, place(original, viewport, &[original], true));
        assert_eq!(original, place(original, viewport, &[original], false));
    }

    #[test]
    fn transparent_colors_and_zero_opacity_have_no_coverage() {
        assert_eq!(color([150, 200, 240, 0], 1.0).a(), 0);
        assert_eq!(color(visual::INK, 0.0).a(), 0);
    }

    fn fixture() -> DrawLabel {
        DrawLabel {
            text: "Sensor α / 1.0 m".into(), at: [0.5, 0.5], color: visual::INK,
            background: [0; 4], accent: visual::ACCENT, size_px: 16.0,
            centred: true, opacity: 1.0, annotation: Default::default(),
        }
    }

    #[test]
    fn shared_host_layout_keeps_panels_and_attached_labels_off_fixed_text() {
        for size in [egui::vec2(1000.0, 700.0), egui::vec2(650.0, 900.0)] {
            let ctx = egui::Context::default();
            let viewport = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), size);
            let mut world = bevy::prelude::World::default();
            let mut fixed = fixture(); fixed.text = "Fixed world label".into();
            let mut selected = fixture(); selected.text = "Selected robot".into(); selected.centred = false;
            selected.annotation.selected = true;
            let mut ordinary = fixture(); ordinary.text = "Attached frame".into(); ordinary.centred = false;
            ordinary.annotation.icon = "frame".into();
            world.insert_resource(crate::draw::DrawLabels(vec![ordinary, selected, fixed]));
            world.insert_resource(crate::draw::DrawPanels(vec![crate::draw::DrawPanel {
                key: ("t".into(), "p".into()), chart: crate::draw::PanelChart {
                    kind: gearbox_api::panel_kind::GAUGE, title: "Range panel".into(), color: [200, 80, 160, 255],
                    values: vec![0.65], ..Default::default()
                }, at: Some([0.5, 0.5]), opacity: 1.0,
            }]));
            let input = egui::RawInput { screen_rect: Some(viewport), ..Default::default() };
            for _ in 0..2 { let _ = crate::host::run_ui_clean(&ctx, input.clone(), |ui| super::super::draw_annotations::queue(ui.ctx(), viewport, &world, &[])); }
            let output = crate::host::run_ui_clean(&ctx, input, |ui| super::super::draw_annotations::queue(ui.ctx(), viewport, &world, &[]));
            let frames: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) if mesh.vertices.iter().any(|v| v.color.a() > 0) => Some(mesh.calc_bounds()),
                _ => None,
            }).collect();
            assert_eq!(frames.len(), 2);
            assert!(!frames[0].intersects(frames[1]));
            for name in ["Fixed world label", "Selected robot", "Attached frame", "Range panel"] {
                let texts: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.override_text_color.is_none() && t.galley.job.text == name => Some(t),
                    _ => None,
                }).collect();
                assert_eq!(texts.len(), 1, "{name}");
                let bounds = egui::Rect::from_min_size(texts[0].pos, texts[0].galley.size());
                assert!(viewport.contains_rect(bounds), "{name}: {bounds:?}");
                if matches!(name, "Fixed world label" | "Attached frame") {
                    assert!(frames.iter().all(|frame| !frame.intersects(bounds)), "{name} is behind glass");
                }
                if name == "Fixed world label" { assert!(bounds.center().distance(viewport.center()) < 0.1); }
            }
        }
    }

    #[test]
    fn movable_annotations_respect_reserved_instance_chrome() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
        let reserved = egui::Rect::from_min_size(egui::pos2(300.0, 8.0), egui::vec2(300.0, 35.0));
        let mut world = bevy::prelude::World::default();
        let mut selected = fixture(); selected.at = [0.5, 0.04]; selected.centred = false;
        selected.text = "Selected robot".into(); selected.annotation.selected = true;
        let mut attached = selected.clone(); attached.text = "Attached frame".into(); attached.annotation.selected = false;
        world.insert_resource(crate::draw::DrawLabels(vec![selected, attached]));
        world.insert_resource(crate::draw::DrawPanels(vec![crate::draw::DrawPanel {
            key: ("test".into(), "table".into()), at: Some([0.5, 0.24]), opacity: 1.0,
            chart: crate::draw::PanelChart { kind: gearbox_api::panel_kind::TABLE, title: "Inspection table".into(),
                rows: vec![("Source".into(), "Deterministic fixture".into()); 5], color: visual::ACCENT, ..Default::default() },
        }]));
        let input = egui::RawInput { screen_rect: Some(viewport), ..Default::default() };
        for _ in 0..3 {
            let output = crate::host::run_ui_clean(&ctx, input.clone(), |ui| super::super::draw_annotations::queue(ui.ctx(), viewport, &world, &[reserved]));
            for name in ["Selected robot", "Attached frame", "Inspection table"] {
                let text = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.galley.text() == name => Some(t), _ => None,
                }).unwrap();
                let bounds = text.galley.rect.translate(text.pos.to_vec2());
                assert!(!reserved.intersects(bounds), "{name}: {bounds:?}");
                assert!(viewport.contains_rect(bounds));
            }
            for shape in &output.shapes {
                if let egui::Shape::Mesh(mesh) = &shape.shape { assert!(!reserved.intersects(mesh.calc_bounds())); }
            }
        }
    }

    #[test]
    fn transparent_and_offscreen_labels_emit_no_geometry() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let mut alpha = fixture(); alpha.color[3] = 0;
        let mut opacity = fixture(); opacity.opacity = 0.0;
        let mut outside = fixture(); outside.at[0] = 1.1;
        let mut invalid = fixture(); invalid.at[1] = f32::NAN;
        let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| paint(ui.ctx(), viewport, &[alpha.clone(), opacity.clone(), outside.clone(), invalid.clone()]));
        assert!(output.shapes.iter().all(|shape| matches!(shape.shape, egui::Shape::Noop)));
    }

    #[test]
    fn glass_backed_labels_use_the_same_single_pass_text_as_panels() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let mut label = fixture();
        label.annotation.selected = true;
        let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| paint(ui.ctx(), viewport, &[label.clone()]));
        let texts: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text), _ => None,
        }).collect();
        assert!(!texts.is_empty());
        assert!(texts.iter().all(|text| text.override_text_color.is_none()));
        assert_eq!(texts.iter().filter(|text| text.galley.job.text == label.text).count(), 1);
    }

    #[test]
    fn selected_label_honors_explicit_transparent_background() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let mut label = fixture();
        label.annotation.selected = true;
        label.annotation.background_explicit = true;
        let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| paint(ui.ctx(), viewport, &[label.clone()]));
        assert!(output.shapes.iter().all(|shape| !matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill.a() > 0)));
        assert!(output.shapes.iter().all(|shape| !matches!(&shape.shape, egui::Shape::Path(path) if path.fill.a() > 0)));
        assert!(output.shapes.iter().any(|shape| matches!(shape.shape, egui::Shape::Text(_))));
    }

    #[test]
    fn secondary_annotations_are_light_neutral_and_keep_identity_tint() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
        let mut label = fixture();
        label.color = [210, 60, 160, 180];
        label.annotation.detail = "score 0.94 / track #7".into();
        let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| paint(ui.ctx(), viewport, &[label.clone()]));
        for (value, expected) in [
            (&label.text, color(label.color, label.opacity)),
            (&label.annotation.detail, color(visual::SECONDARY, label.opacity * 180.0 / 255.0)),
        ] {
            let text = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.override_text_color.is_none() && t.galley.job.text == *value => Some(t),
                _ => None,
            }).unwrap();
            assert_eq!(text.galley.job.sections[0].format.color, expected);
        }
        assert!(visual::SECONDARY[..3].iter().all(|channel| *channel >= 190));
        assert!(visual::SECONDARY[2] - visual::SECONDARY[0] <= 20);
    }

    #[test]
    fn unbacked_text_uses_opposite_edges_without_changing_caller_ink() {
        for (rgb, light_edge) in [([0, 0, 0], true), ([0, 114, 178], true), ([40, 25, 35], true),
            ([224, 239, 247], false), ([255, 220, 50], false), ([180, 180, 180], false)] {
            for size in [13.0, 16.0] {
                let ctx = egui::Context::default();
                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
                let mut label = fixture();
                label.color = [rgb[0], rgb[1], rgb[2], 255];
                label.size_px = size;
                let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| paint(ui.ctx(), viewport, &[label.clone()]));
                let texts: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t), _ => None,
                }).collect();
                let main = texts.iter().find(|t| t.override_text_color.is_none()).unwrap();
                assert_eq!(main.galley.job.sections[0].format.color, color(label.color, 1.0));
                let edges: Vec<_> = texts.iter().filter(|t| t.override_text_color.is_some()).collect();
                assert_eq!(edges.len(), 4);
                for edge in edges {
                    let key = edge.override_text_color.unwrap().to_srgba_unmultiplied();
                    assert_eq!(key[3], 175);
                    assert_eq!(key[0] > 200, light_edge, "{rgb:?} edge {key:?}");
                    let offset = edge.pos - main.pos;
                    assert!((offset.length() - if size < 14.0 { 0.5 } else { 0.75 }).abs() < 0.001);
                }
            }
        }
    }

    #[test]
    fn text_edge_polarity_stays_stable_while_opacity_fades() {
        for gray in 108..=115 {
            let source = [gray, gray, gray, 255];
            let full = text_edge_color(color(source, 1.0), source);
            for alpha in 64..=255 {
                let faded = text_edge_color(color(source, alpha as f32 / 255.0), source);
                assert_eq!(faded.r() > 0, full.r() > 0, "gray {gray}, alpha {alpha}");
            }
        }
    }

    #[test]
    fn category_edges_use_category_alpha_instead_of_identity_alpha() {
        for alpha in [0, 64, 128, 255] {
            for opacity in [0.25, 0.5, 1.0] {
                let ctx = egui::Context::default();
                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
                let mut label = fixture();
                label.annotation.category = "CATEGORY".into();
                label.accent = [0, 114, 178, alpha];
                label.opacity = opacity;
                let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| paint(ui.ctx(), viewport, &[label.clone()]));
                let ink = color(label.accent, opacity);
                let expected_alpha = (ink.a() as f32 * (175.0 / 255.0)) as u8;
                let mut visible_edges = 0;
                let mut visible_ink = 0;
                for shape in output.shapes {
                    if let egui::Shape::Text(t) = shape.shape && t.galley.job.text == "CATEGORY" {
                        if let Some(edge) = t.override_text_color {
                            assert_eq!(edge.a(), expected_alpha, "category alpha {alpha}, opacity {opacity}");
                            visible_edges += usize::from(edge.a() > 0);
                        } else {
                            assert_eq!(t.galley.job.sections[0].format.color, ink);
                            visible_ink += usize::from(ink.a() > 0);
                        }
                    }
                }
                assert_eq!(visible_edges, if expected_alpha > 0 { 4 } else { 0 });
                assert_eq!(visible_ink, usize::from(ink.a() > 0));
            }
        }
    }

    #[test]
    fn world_fonts_leave_shell_families_and_unicode_fallbacks_intact() {
        let ctx = egui::Context::default();
        let mut original = Vec::new();
        let _ = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
            let ctx = ui.ctx();
            original = ctx.fonts(|f| f.definitions().families[&egui::FontFamily::Proportional].clone());
            font(ctx, 16.0, true);
        });
        let _ = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
            let ctx = ui.ctx();
            assert_eq!(font(ctx, 16.0, true).family, egui::FontFamily::Name(IDENTITY.into()));
            ctx.fonts(|f| {
                assert_eq!(f.definitions().families[&egui::FontFamily::Proportional], original);
                for name in [IDENTITY, BODY] {
                    let list = &f.definitions().families[&egui::FontFamily::Name(name.into())];
                    assert_eq!(list[0], name);
                    assert!(original.iter().all(|fallback| list.contains(fallback)));
                }
            });
        });
    }

    #[test]
    fn overprinting_anchored_labels_keep_the_nearer_one() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut far = fixture();
        far.text = "03 / EXACT CLASSES".into();
        far.size_px = 12.0;
        let mut near = fixture();
        near.text = "04 / MEASURED RGB".into();
        near.size_px = 18.0;
        near.at = [0.51, 0.5];
        let mut apart = fixture();
        apart.text = "01 / AGE VIRIDIS".into();
        apart.at = [0.2, 0.2];
        let labels = [far, near, apart];
        let _ = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
            let prepared = prepare(ui.ctx(), viewport, &labels);
            let shown: Vec<_> = prepared
                .layouts
                .iter()
                .filter(|layout| !layout.hidden)
                .map(|layout| layout.label.text.as_str())
                .collect();
            assert_eq!(shown, vec!["04 / MEASURED RGB", "01 / AGE VIRIDIS"]);
            assert!(prepared.layouts.iter().any(|layout| layout.hidden && layout.label.size_px == 12.0));
        });
    }
}
