//! Reusable egui widgets shared across editor panels.
//!
//! These compose against the design-system tokens in [`super::style`]
//! so every panel reads with the same typographic + spacing rhythm.
//! Keep them purely presentational — they should not know about
//! `Selection`, `PendingSpawn`, or any domain state. Pass plain values
//! (strings, colours, closures) in and let the caller react to the
//! returned `Response`.

use bevy_egui::egui;

use super::style::{
    body_label, caption, glass_alpha_card, glass_alpha_group, glass_fill, radius, section_caps,
    space, BG_2_RAISED, BG_3_HOVER, BORDER_SUBTLE, TEXT_PRIMARY, TEXT_SECONDARY,
};

// ─── Section ────────────────────────────────────────────────────────
//
// The canonical collapsible block. Used everywhere a panel has a
// headline + body.
//   [HEADER]                (accent-coloured UPPERCASE)
//   ──────── 1 px divider
//   body goes here, inset by default egui indent
//
// Uses `CollapsingHeader` under the hood, so the ▶/▼ chevron still
// animates. The divider appears only when the section is expanded.

pub fn section(
    ui: &mut egui::Ui,
    id_salt: &str,
    title: &str,
    accent: egui::Color32,
    default_open: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    // Each section lives in its own subtle card pinned to the panel's
    // full available width. Card width never grows with body content:
    //
    //   1. Capture `full_w` BEFORE opening the Frame.
    //   2. Open the Frame and inside it, wrap the header + body in an
    //      explicit fixed-width child via `allocate_ui_with_layout`.
    //   3. Install a `set_clip_rect` matching that child's rect so any
    //      widget that tries to draw past the card edge gets clipped
    //      rather than pushing the card wider (the failure mode you
    //      saw when expanding Power: a wide internal widget was
    //      stretching the Frame's bounding rect).
    let full_w = ui.available_width();
    let inner_w = (full_w - 18.0).max(0.0); // 8 px × 2 padding + 2 stroke
    egui::Frame::new()
        .fill(glass_fill(BG_2_RAISED, accent, glass_alpha_card()))
        .corner_radius(egui::CornerRadius::same(radius::MD))
        .stroke(egui::Stroke::new(1.0, BORDER_SUBTLE))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(inner_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(inner_w);
                    // Hard clip: anything trying to draw past `inner_w`
                    // on the right (long text, unconstrained DragValue,
                    // whatever) gets visually cut instead of bulging
                    // the card. We intersect with the current clip so
                    // parent scroll / window clips still apply.
                    let clip = ui.clip_rect().intersect(egui::Rect::from_min_size(
                        ui.min_rect().min,
                        egui::vec2(inner_w, f32::INFINITY),
                    ));
                    ui.set_clip_rect(clip);
                    egui::CollapsingHeader::new(section_caps(title, accent))
                        .id_salt(id_salt)
                        .default_open(default_open)
                        // `.show_unindented` — the frame already pads
                        // the body, so the default 14 px indent would
                        // double up and misalign rows across panels.
                        .show_unindented(ui, body);
                },
            );
        });
}

// ─── Labelled row ───────────────────────────────────────────────────
//
// Label on the left, control(s) right-aligned on the right. The label
// cell is a **fixed** width and truncates overlong text with `…`, so
// the column of controls stays vertically aligned across rows
// regardless of which row has the longest label. That's what kept
// the previous version looking ragged — labels like "max brake (N·m
// /wheel)" shoved the control cell around.
//
// The right cell takes whatever width remains and right-aligns its
// content, so DragValues and ComboBoxes hug the panel edge uniformly.

/// Width of the label column. Picked to fit every current label at
/// the 11 pt body size; anything wider truncates.
const LABEL_COL_WIDTH: f32 = 140.0;

pub fn labelled_row(
    ui: &mut egui::Ui,
    label: &str,
    right: impl FnOnce(&mut egui::Ui),
) {
    labelled_row_custom_left(
        ui,
        |ui| {
            ui.add(egui::Label::new(body_label(label)).truncate());
        },
        right,
    );
}

/// Same row skeleton as [`labelled_row`] (fixed-width left cell,
/// right-aligned right cell with strict max width) but the left cell
/// is rendered by a caller-supplied closure. Used by axis rows that
/// want a coloured `X` / `Y` / `Z` glyph in the label slot and by any
/// future row that needs a non-trivial label (icon + text, chip, etc.).
pub fn labelled_row_custom_left(
    ui: &mut egui::Ui,
    left: impl FnOnce(&mut egui::Ui),
    right: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        let row_h = ui.spacing().interact_size.y;
        ui.allocate_ui_with_layout(
            egui::vec2(LABEL_COL_WIDTH, row_h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                left(ui);
            },
        );
        let remaining = ui.available_width().max(0.0);
        ui.allocate_ui_with_layout(
            egui::vec2(remaining, row_h),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.set_max_width(remaining);
                right(ui);
            },
        );
    });
}

/// Read-only numeric/text row: label left, monospaced value right.
pub fn readout_row(ui: &mut egui::Ui, label: &str, value: &str) {
    labelled_row(ui, label, |ui| {
        ui.label(
            egui::RichText::new(value)
                .monospace()
                .small()
                .color(TEXT_PRIMARY),
        );
    });
}

/// Coloured-glyph + value row (e.g. `X  +1.234 m` in AXIS_X). Used
/// by transform/position/rotation readouts in the Inspector. The
/// glyph sits in the same left-cell as `labelled_row`'s label so the
/// whole panel aligns on one vertical line.
pub fn axis_readout_row(
    ui: &mut egui::Ui,
    glyph: &str,
    glyph_color: egui::Color32,
    value: &str,
) {
    ui.horizontal(|ui| {
        let row_h = ui.spacing().interact_size.y;
        ui.allocate_ui_with_layout(
            egui::vec2(LABEL_COL_WIDTH, row_h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(glyph)
                        .strong()
                        .monospace()
                        .small()
                        .color(glyph_color),
                );
            },
        );
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(value)
                        .monospace()
                        .small()
                        .color(TEXT_PRIMARY),
                );
            },
        );
    });
}

// ─── Small helpers ──────────────────────────────────────────────────

/// Subtle caption text (italic, small, tertiary colour). Use between
/// related sub-blocks inside a section to describe what follows.
pub fn sub_caption(ui: &mut egui::Ui, text: &str) {
    ui.label(caption(text));
}

/// A chunky primary-looking button that fills the available row width.
/// Used for "Refuel / Repower" etc. Carries a subtle accent tint at
/// rest (≈ 8 % of accent over the raised panel colour), brightens on
/// hover / press, and paints an accent border on hover — so the user's
/// eye can tell it's interactive at a glance without the button
/// screaming for attention.
pub fn wide_button(ui: &mut egui::Ui, label: &str, accent: egui::Color32) -> egui::Response {
    const ROW_H: f32 = 24.0;
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(w, ROW_H),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        paint_accent_bg(ui, rect, accent, &resp);
        ui.painter_at(rect).text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.0),
            TEXT_PRIMARY,
        );
    }
    resp
}

// ─── Card button ────────────────────────────────────────────────────
//
// A full-width preset card — accent glyph on the left, primary name +
// small subtitle on the right. Reads like UE5's "Create" entries.

pub fn card_button(
    ui: &mut egui::Ui,
    glyph: &str,
    name: &str,
    subtitle: &str,
    accent: egui::Color32,
) -> egui::Response {
    // Geometry constants. Reserving the SAME amount of blank space on
    // the right as the glyph column consumes on the left keeps the
    // name / subtitle text optically centred in the card and leaves a
    // clean "runway" where the ellipsis appears when either line is
    // too long to fit — rather than text bleeding into the card's
    // rounded-corner on the right.
    const ROW_H:        f32 = 32.0;
    const EDGE_PAD:     f32 = 8.0;   // from card edge to content
    const GLYPH_COL:    f32 = 14.0;  // glyph bbox from start of content
    const GLYPH_GAP:    f32 = 8.0;   // glyph-to-text gap
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(w, ROW_H),
        egui::Sense::click(),
    );
    if !ui.is_rect_visible(rect) {
        return resp;
    }

    paint_accent_bg(ui, rect, accent, &resp);

    let painter = ui.painter_at(rect);

    // Glyph pinned to the left.
    let glyph_x = rect.min.x + EDGE_PAD + GLYPH_COL * 0.5;
    painter.text(
        egui::pos2(glyph_x, rect.center().y),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(14.0),
        accent,
    );

    // Text column starts past the glyph and ENDS at a mirrored
    // padding on the right — same gutter as the glyph column, so
    // the whole card reads symmetrically.
    let text_left  = rect.min.x + EDGE_PAD + GLYPH_COL + GLYPH_GAP;
    let text_right = rect.max.x - (EDGE_PAD + GLYPH_COL + GLYPH_GAP);
    let max_w = (text_right - text_left).max(0.0);

    let name_galley = elided_galley(
        ui,
        name,
        egui::FontId::proportional(12.0),
        TEXT_PRIMARY,
        max_w,
    );
    let subtitle_galley = elided_galley(
        ui,
        subtitle,
        egui::FontId::proportional(10.0),
        TEXT_SECONDARY,
        max_w,
    );

    // Vertically stack name (upper) / subtitle (lower) on the card's
    // centreline. Galleys report their own height so we position each
    // so its vertical centre sits at the expected y.
    let center_y = rect.center().y;
    let name_pos = egui::pos2(
        text_left,
        center_y - 6.0 - name_galley.size().y * 0.5,
    );
    let subtitle_pos = egui::pos2(
        text_left,
        center_y + 7.0 - subtitle_galley.size().y * 0.5,
    );
    painter.galley(name_pos, name_galley, TEXT_PRIMARY);
    painter.galley(subtitle_pos, subtitle_galley, TEXT_SECONDARY);

    resp
}

/// Paint a rounded-rect button background with a tiny accent tint.
/// Shared by `card_button` + `wide_button` so they read as one family.
fn paint_accent_bg(
    ui: &egui::Ui,
    rect: egui::Rect,
    accent: egui::Color32,
    resp: &egui::Response,
) {
    let tint = if resp.is_pointer_button_down_on() {
        0.30
    } else if resp.hovered() {
        0.16
    } else {
        0.08
    };
    // Preserve the glass alpha so card/wide buttons blend into the
    // panel behind them the same way the card frames do. Unmultiplied
    // so low alphas read as "mostly scene + tiny surface tint" rather
    // than "gray block with partial transparency added on top".
    let solid = lerp_color(BG_2_RAISED, accent, tint);
    let bg = egui::Color32::from_rgba_unmultiplied(
        solid.r(),
        solid.g(),
        solid.b(),
        glass_alpha_card(),
    );
    let border_col = if resp.hovered() { accent } else { BORDER_SUBTLE };
    ui.painter_at(rect).rect(
        rect,
        egui::CornerRadius::same(radius::MD),
        bg,
        egui::Stroke::new(1.0, border_col),
        egui::StrokeKind::Inside,
    );
}

/// Lay out `text` onto a single row, truncated with `…` when it would
/// otherwise exceed `max_w`. Wrap settings mirror egui's own
/// `Label::truncate()` behaviour.
fn elided_galley(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat::simple(font, color),
    );
    job.wrap.max_width = max_w;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.halign = egui::Align::LEFT;
    ui.painter().layout_job(job)
}

// ─── Pill toggle (iOS-style on/off switch) ──────────────────────────
//
// Custom-painted because egui's default `checkbox` renders a tiny
// square with a tick mark — too visually noisy for header-level
// "power ON / OFF" toggles. The pill has a clear state (filled accent
// = on, dim grey = off) and a sliding knob that animates between.

const TOGGLE_WIDTH: f32 = 28.0;
const TOGGLE_HEIGHT: f32 = 14.0;

pub fn toggle(ui: &mut egui::Ui, on: &mut bool, accent: egui::Color32) -> egui::Response {
    let desired = egui::vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(desired, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, "")
    });

    if ui.is_rect_visible(rect) {
        // Smooth animation between states — egui remembers the
        // in-flight value by response id so repeated draws fade.
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);

        let bg_off = ui.visuals().widgets.inactive.bg_fill;
        let bg_on  = accent;
        let bg = lerp_color(bg_off, bg_on, how_on);
        let stroke = egui::Stroke::new(1.0, BORDER_SUBTLE);
        let radius_px = rect.height() * 0.5;

        ui.painter().rect(
            rect,
            radius_px,
            bg,
            stroke,
            egui::StrokeKind::Inside,
        );

        let knob_r = radius_px - 2.0;
        let knob_x = egui::lerp(
            (rect.left() + radius_px)..=(rect.right() - radius_px),
            how_on,
        );
        ui.painter().circle_filled(
            egui::pos2(knob_x, rect.center().y),
            knob_r,
            egui::Color32::WHITE,
        );
    }
    response
}

fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| ((x as f32) * (1.0 - t) + (y as f32) * t).round() as u8;
    egui::Color32::from_rgba_premultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
}

// ─── Pretty slider ──────────────────────────────────────────────────
//
// egui's built-in `Slider` draws a thin line with a tiny circle —
// functional but a bit utilitarian. This is a custom-painted version
// that feels closer to modern audio / video progress sliders:
//   - fat rounded pill track (6 px tall) with a dark "unfilled" half
//     and an accent-coloured "filled" portion,
//   - a crisp circular knob at the fill edge that lights up on hover,
//   - value readout on the right lives in a `DragValue` so double-
//     click-to-type still works like on the stock egui slider.
//
// The track is the interactable region; clicking or dragging inside
// it sets the value proportionally. The DragValue handles keyboard +
// typed-entry. `changed()` is returned so callers can flush to the
// simulator.

const SLIDER_TRACK_H:   f32 = 6.0;
/// Knob is a vertical bar ("I") rather than a circle. Half-width
/// in pixels — used to compute the knob's rect centred on the fill
/// edge. Slightly wider than a 1 px line so it reads as a proper
/// grab-handle rather than a tick mark.
const SLIDER_KNOB_HALF_W: f32 = 1.5;
/// Knob extends above + below the track so it looks like a distinct
/// control element, not a fat line.
const SLIDER_KNOB_OVERHANG: f32 = 4.0;

pub fn pretty_slider(
    ui: &mut egui::Ui,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    decimals: usize,
    suffix: &str,
    accent: egui::Color32,
) -> egui::Response {
    ui.horizontal(|ui| {
        let track_w = ui.spacing().slider_width;
        let row_h = ui.spacing().interact_size.y;

        let (rect, mut resp) = ui.allocate_exact_size(
            egui::vec2(track_w, row_h),
            egui::Sense::click_and_drag(),
        );

        let (lo, hi) = (*range.start(), *range.end());
        let denom = (hi - lo).max(f64::EPSILON);

        // Interaction — click/drag on the track sets value.
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.dragged() || resp.clicked() {
                let new_t = ((pos.x - rect.min.x) as f64 / rect.width() as f64).clamp(0.0, 1.0);
                let new_val = lo + new_t * denom;
                if (new_val - *value).abs() > f64::EPSILON {
                    *value = new_val.clamp(lo, hi);
                    resp.mark_changed();
                }
            }
        }

        // Paint.
        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect);
            let center_y = rect.center().y;
            let track = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, center_y - SLIDER_TRACK_H * 0.5),
                egui::vec2(track_w, SLIDER_TRACK_H),
            );
            let radius = SLIDER_TRACK_H * 0.5;

            // Unfilled background
            painter.rect_filled(
                track,
                egui::CornerRadius::same(radius as u8),
                ui.visuals().extreme_bg_color,
            );

            // Filled portion from start to current value
            let t = ((*value - lo) / denom).clamp(0.0, 1.0) as f32;
            let fill_w = track_w * t;
            if fill_w > 0.5 {
                let fill = egui::Rect::from_min_size(
                    track.min,
                    egui::vec2(fill_w, SLIDER_TRACK_H),
                );
                painter.rect_filled(
                    fill,
                    egui::CornerRadius::same(radius as u8),
                    accent,
                );
            }

            // Knob at the fill edge — an "I" bar that extends a few px
            // above + below the track. At 0 % the bar is centred on the
            // track's left edge and at 100 % on the right edge; because
            // `painter` is clipped to the slider's allocation rect, the
            // outside half gets sliced off cleanly — giving a subtle
            // "nuzzled into the end" feel rather than a circle hanging
            // off the track.
            let (knob_half_w, knob_fill) = if resp.dragged() {
                (SLIDER_KNOB_HALF_W + 1.0, egui::Color32::WHITE)
            } else if resp.hovered() {
                (SLIDER_KNOB_HALF_W + 0.5, egui::Color32::WHITE)
            } else {
                (SLIDER_KNOB_HALF_W, egui::Color32::from_rgb(0xEC, 0xEC, 0xEE))
            };
            let knob_cx = track.min.x + fill_w;
            let knob_rect = egui::Rect::from_min_max(
                egui::pos2(knob_cx - knob_half_w, track.min.y - SLIDER_KNOB_OVERHANG),
                egui::pos2(knob_cx + knob_half_w, track.max.y + SLIDER_KNOB_OVERHANG),
            );
            painter.rect_filled(
                knob_rect,
                egui::CornerRadius::same(1),
                knob_fill,
            );
        }

        // Value display — DragValue keeps double-click-to-type working.
        ui.add_space(space::TIGHT);
        let mut v = *value;
        let drag = ui.add(
            egui::DragValue::new(&mut v)
                .speed(0.0) // drag behaviour is owned by the track
                .range(lo..=hi)
                .fixed_decimals(decimals)
                .suffix(suffix),
        );
        if drag.changed() {
            *value = v.clamp(lo, hi);
            resp.mark_changed();
        }

        resp
    })
    .inner
}

// ─── Group frame ────────────────────────────────────────────────────
//
// A subtle rounded rectangle that groups a small cluster of controls
// (e.g. the primary-source radio group, a refuel button + its hint).
// Much lighter than a new collapsible header — it just says "these
// belong together" without another fold-level nesting.

pub fn group_frame(
    ui: &mut egui::Ui,
    accent: egui::Color32,
    body: impl FnOnce(&mut egui::Ui),
) {
    // Same glass recipe as the section card but slightly denser so
    // the nested group reads as "one more layer in" than the card
    // around it. Uses `BG_3_HOVER` as the base so groups sit a
    // touch brighter than cards, reinforcing the stacked-pane feel.
    egui::Frame::new()
        .fill(glass_fill(BG_3_HOVER, accent, glass_alpha_group()))
        .corner_radius(egui::CornerRadius::same(radius::SM))
        .stroke(egui::Stroke::new(1.0, BORDER_SUBTLE))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, body);
}

/// Key-chip + label row used in the "Keys" help section. Action text
/// truncates with `…` if the full line would overflow the card.
pub fn keybinding_row(ui: &mut egui::Ui, keys: &str, action: &str) {
    ui.horizontal(|ui| {
        let chip = egui::RichText::new(keys)
            .monospace()
            .small()
            .color(ui.visuals().text_color());
        let frame = egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .inner_margin(egui::Margin::symmetric(5, 1))
            .corner_radius(egui::CornerRadius::same(radius::SM));
        frame.show(ui, |ui| ui.label(chip));
        ui.add_space(space::TIGHT);
        ui.add(
            egui::Label::new(
                egui::RichText::new(action).small().color(TEXT_SECONDARY),
            )
            .truncate(),
        );
    });
}
