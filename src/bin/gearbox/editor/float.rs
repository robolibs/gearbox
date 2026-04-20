//! Helpers for drawing the floating UI.
//!
//! Layout constants are centralised here so the left and right docks look
//! identical (rail width, margins, gap to panel, letter size).

use bevy_egui::egui;

use super::style::accent_color;

// --- layout constants ---------------------------------------------------
/// Edge length of each square side button.
pub const SIDE_BTN_SIZE: f32 = 42.0;
/// Vertical gap between stacked side buttons.
pub const SIDE_BTN_GAP: f32 = 6.0;
/// Distance from the screen edge to the near edge of each button.
pub const EDGE_GAP: f32 = 10.0;
/// Gap between a side button and the opened panel.
pub const RAIL_PANEL_GAP: f32 = 8.0;

// --- colours ------------------------------------------------------------
const RAIL_BG:     egui::Color32 = egui::Color32::from_rgb(0x17, 0x19, 0x1C);
const RAIL_STROKE: egui::Color32 = egui::Color32::from_rgb(0x2E, 0x31, 0x38);
const ICON_FG:     egui::Color32 = egui::Color32::from_rgb(0xB0, 0xB4, 0xBC);

// --- helpers ------------------------------------------------------------

/// Standalone side button — just a single styled button in its own
/// `Area`, no wrapping rail, no frame-in-frame nonsense. Mirrors the
/// `.panel-toggle` style from coreviz/deck: a rounded square with its
/// own shadow, positioned by slot index from the anchored corner.
pub fn side_button(
    id: &'static str,
    ctx: &egui::Context,
    anchor: egui::Align2,
    slot: u32,
    glyph: &str,
    tooltip: &str,
    is_active: bool,
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
            icon_button(ui, glyph, tooltip, is_active, on_click);
        });
}

/// A single rounded-square letter button with shadow and hover — this
/// is the ONLY box. No outer rail.
pub fn icon_button(
    ui: &mut egui::Ui,
    glyph: &str,
    tooltip: &str,
    is_active: bool,
    on_click: impl FnOnce(),
) {
    let (bg, fg, stroke) = if is_active {
        (
            egui::Color32::from_rgb(0x26, 0x30, 0x3A),
            accent_color(),
            egui::Color32::from_rgb(0x42, 0x52, 0x62),
        )
    } else {
        (RAIL_BG, ICON_FG, RAIL_STROKE)
    };
    let text = egui::RichText::new(glyph)
        .size(16.0)
        .strong()
        .family(egui::FontFamily::Monospace)
        .color(fg);
    let btn = egui::Button::new(text)
        .corner_radius(egui::CornerRadius::same(10))
        .fill(bg)
        .stroke(egui::Stroke::new(1.0, stroke));
    let resp = ui.add_sized([SIDE_BTN_SIZE, SIDE_BTN_SIZE], btn);
    if resp.on_hover_text(tooltip).clicked() {
        on_click();
    }
}

/// Floating content panel anchored to a screen corner with
/// `SIDE_BTN_SIZE + RAIL_PANEL_GAP` of clearance so it never overlaps
/// the side button. No close button — the side button toggles it.
///
/// The panel is **fixed size** (taken from `size`). It does not
/// auto-resize to fit content: content is capped via `set_max_width`,
/// and the window is neither resizable nor grows with response rect.
pub fn floating_window(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    anchor: egui::Align2,
    size: egui::Vec2,
    _open: &mut bool,
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
        inner_margin: egui::Margin { left: 12, right: 12, top: 10, bottom: 10 },
        outer_margin: egui::Margin::ZERO,
        fill: egui::Color32::from_rgb(0x1E, 0x20, 0x24),
        stroke: egui::Stroke::new(1.0, RAIL_STROKE),
        corner_radius: egui::CornerRadius::same(10),
        shadow: egui::epaint::Shadow {
            offset: [0, 6], blur: 18, spread: 0,
            color: egui::Color32::from_black_alpha(160),
        },
    };

    egui::Window::new(title)
        .id(egui::Id::new(id))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(anchor, anchor_offset)
        .fixed_size(size)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_max_width(size.x - 24.0);
            ui.label(
                egui::RichText::new(title)
                    .heading()
                    .color(accent_color()),
            );
            ui.separator();
            add_contents(ui);
        });
}
