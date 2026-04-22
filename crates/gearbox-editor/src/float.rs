//! Helpers for the floating dock UI: standalone side buttons and
//! fixed-size content panels.

use bevy_egui::egui;

use super::style::{
    glass_fill, BG_1_PANEL, BG_2_RAISED, BORDER_SUBTLE, GLASS_ALPHA_CARD, GLASS_ALPHA_WINDOW,
    TEXT_PRIMARY, TEXT_SECONDARY,
};

/// Shared background / border recipe for every rail button (left side
/// tabs, top transport glyphs). Same glass look as the main panels —
/// [`BG_1_PANEL`] idle, lifts to [`BG_2_RAISED`] on hover, and 25 %
/// accent blend + accent stroke when active.
fn paint_rail_button(
    painter: &egui::Painter,
    rect: egui::Rect,
    accent: egui::Color32,
    is_active: bool,
    hovered: bool,
) {
    // Idle / hover both use the window-level glass alpha so the
    // buttons float on the background the same way the main panels
    // do. The "active" state blends 25 % accent into the card-level
    // base, then applies the card alpha — slightly denser than the
    // idle buttons so it reads as the selected tab.
    let bg = if is_active {
        let blend = |a: u8, b: u8| ((a as f32) * 0.75 + (b as f32) * 0.25).round() as u8;
        let tinted = egui::Color32::from_rgb(
            blend(BG_2_RAISED.r(), accent.r()),
            blend(BG_2_RAISED.g(), accent.g()),
            blend(BG_2_RAISED.b(), accent.b()),
        );
        glass_fill(tinted, accent, GLASS_ALPHA_WINDOW)
    } else if hovered {
        glass_fill(BG_2_RAISED, accent, GLASS_ALPHA_WINDOW)
    } else {
        glass_fill(BG_1_PANEL, accent, GLASS_ALPHA_WINDOW)
    };
    let stroke = if is_active { accent } else { BORDER_SUBTLE };
    painter.rect_filled(rect, egui::CornerRadius::same(6), bg);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(6),
        egui::Stroke::new(1.0, stroke),
        egui::StrokeKind::Outside,
    );
    let _ = GLASS_ALPHA_CARD; // kept so callers that tune the "active" density can switch
}

// --- layout constants ---------------------------------------------------
/// Edge length of each square side button (VS Code / Fleet activity-bar).
pub const SIDE_BTN_SIZE: f32 = 34.0;
/// Vertical gap between stacked side buttons.
pub const SIDE_BTN_GAP: f32 = 4.0;
/// Distance from the screen edge to the near edge of each button.
pub const EDGE_GAP: f32 = 8.0;
/// Gap between a side button and the opened panel.
pub const RAIL_PANEL_GAP: f32 = 6.0;

/// Standalone side button. Single rounded square with an accent
/// left-border when active (VS Code / Fleet convention).
pub fn side_button(
    id: &'static str,
    ctx: &egui::Context,
    anchor: egui::Align2,
    slot: u32,
    glyph: &str,
    tooltip: &str,
    is_active: bool,
    accent: egui::Color32,
    on_click: impl FnOnce(),
) {
    let slot_y = slot as f32 * (SIDE_BTN_SIZE + SIDE_BTN_GAP);
    let offset = match anchor {
        egui::Align2::LEFT_TOP     => egui::vec2( EDGE_GAP,  EDGE_GAP + slot_y),
        egui::Align2::RIGHT_TOP    => egui::vec2(-EDGE_GAP,  EDGE_GAP + slot_y),
        egui::Align2::LEFT_BOTTOM  => egui::vec2( EDGE_GAP, -EDGE_GAP - slot_y),
        egui::Align2::RIGHT_BOTTOM => egui::vec2(-EDGE_GAP, -EDGE_GAP - slot_y),
        _ => egui::vec2(EDGE_GAP, EDGE_GAP + slot_y),
    };

    egui::Area::new(egui::Id::new(id))
        .anchor(anchor, offset)
        .interactable(true)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(
                egui::vec2(SIDE_BTN_SIZE, SIDE_BTN_SIZE),
                egui::Sense::click(),
            );

            paint_rail_button(ui.painter(), rect, accent, is_active, resp.hovered());
            let fg = if is_active { TEXT_PRIMARY } else { TEXT_SECONDARY };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                egui::FontId::new(14.0, egui::FontFamily::Monospace),
                fg,
            );

            if resp.on_hover_text(tooltip).clicked() {
                on_click();
            }
        });
}

/// Top-center button, same visual language as [`side_button`] but
/// anchored to the top edge and laid out horizontally. `slot` is the
/// index within a horizontal row of `count` buttons, 0-indexed from
/// the left.
pub fn top_button(
    id: &'static str,
    ctx: &egui::Context,
    slot: u32,
    count: u32,
    glyph: &str,
    tooltip: &str,
    is_active: bool,
    accent: egui::Color32,
    on_click: impl FnOnce(),
) {
    // Row width with inter-button gaps, centred on 0.
    let n = count.max(1) as f32;
    let step = SIDE_BTN_SIZE + SIDE_BTN_GAP;
    let row_w = n * SIDE_BTN_SIZE + (n - 1.0).max(0.0) * SIDE_BTN_GAP;
    let offset_x = -(row_w - SIDE_BTN_SIZE) * 0.5 + slot as f32 * step;

    egui::Area::new(egui::Id::new(id))
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(offset_x, EDGE_GAP))
        .interactable(true)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(
                egui::vec2(SIDE_BTN_SIZE, SIDE_BTN_SIZE),
                egui::Sense::click(),
            );

            paint_rail_button(ui.painter(), rect, accent, is_active, resp.hovered());
            let fg = if is_active { TEXT_PRIMARY } else { TEXT_SECONDARY };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                egui::FontId::new(14.0, egui::FontFamily::Monospace),
                fg,
            );

            if resp.on_hover_text(tooltip).clicked() {
                on_click();
            }
        });
}

/// Floating content panel anchored to a screen corner. Fixed size
/// (does NOT auto-resize with content); no title bar / close button.
pub fn floating_window(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    anchor: egui::Align2,
    size: egui::Vec2,
    _open: &mut bool,
    accent: egui::Color32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let side_inset = EDGE_GAP + SIDE_BTN_SIZE + RAIL_PANEL_GAP;
    let anchor_offset = match anchor {
        egui::Align2::LEFT_TOP     => egui::vec2( side_inset,  EDGE_GAP),
        egui::Align2::RIGHT_TOP    => egui::vec2(-side_inset,  EDGE_GAP),
        egui::Align2::LEFT_BOTTOM  => egui::vec2( side_inset, -EDGE_GAP),
        egui::Align2::RIGHT_BOTTOM => egui::vec2(-side_inset, -EDGE_GAP),
        _ => egui::vec2(side_inset, EDGE_GAP),
    };

    let frame = egui::Frame {
        inner_margin: egui::Margin { left: 6, right: 6, top: 6, bottom: 8 },
        outer_margin: egui::Margin::ZERO,
        // Glassy panel: slightly transparent + faint accent tint,
        // so selecting a vehicle shifts the entire UI's hue by a
        // hair (purple → vehicle colour).
        fill: glass_fill(BG_1_PANEL, accent, GLASS_ALPHA_WINDOW),
        stroke: egui::Stroke::new(1.0, BORDER_SUBTLE),
        corner_radius: egui::CornerRadius::same(8),
        shadow: egui::epaint::Shadow {
            offset: [0, 8], blur: 24, spread: 0,
            color: egui::Color32::from_black_alpha(115),
        },
    };

    let on_right_side = matches!(
        anchor,
        egui::Align2::RIGHT_TOP | egui::Align2::RIGHT_BOTTOM
    );

    egui::Window::new(title)
        .id(egui::Id::new(id))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(anchor, anchor_offset)
        .fixed_size(size)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_max_width(size.x - 12.0);

            // Title row: UPPERCASE accent, roomy type, with a hairline
            // painted directly beneath and a generous breathing gap
            // before the panel's content starts.
            //
            // 15 pt baseline + 15 % ≈ 17.25 pt — the panel caption sits
            // a clear step above the section headers (12 pt + 15 %)
            // inside the cards below, without feeling shouty.
            let title_size = 15.0 * 1.15;
            let title_h = 25.0;
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), title_h),
                egui::Sense::hover(),
            );
            let (align, tx) = if on_right_side {
                (egui::Align2::RIGHT_CENTER, rect.max.x)
            } else {
                (egui::Align2::LEFT_CENTER, rect.min.x)
            };
            // Fake-bold: `painter.text` has no `.strong()`, so we
            // double-stroke the caption with a 0.5 px horizontal
            // offset to bulk up every stem. Sub-pixel so it reads as
            // "a bit heavier" rather than "a wobbly double image".
            let pos = egui::pos2(tx, rect.center().y);
            let font = egui::FontId::new(title_size, egui::FontFamily::Proportional);
            for dx in [-0.5, 0.5] {
                ui.painter().text(
                    egui::pos2(pos.x + dx, pos.y),
                    align,
                    title.to_uppercase(),
                    font.clone(),
                    accent,
                );
            }
            ui.painter().hline(
                rect.min.x..=rect.max.x,
                rect.max.y + 3.0,
                egui::Stroke::new(1.0, BORDER_SUBTLE),
            );
            ui.add_space(16.0);

            add_contents(ui);
        });
}
