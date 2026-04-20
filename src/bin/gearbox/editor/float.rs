//! Helpers for drawing the floating UI.
//!
//! Layout constants are centralised here so the left and right docks look
//! identical (rail width, margins, gap to panel, letter size).

use bevy_egui::egui;

use super::style::accent_color;

// --- layout constants ---------------------------------------------------
/// Side of each square icon button.
pub const BTN: f32 = 34.0;
/// Total rail width (button + side padding + 1 px stroke on each side).
pub const RAIL_W: f32 = BTN + 10.0 + 2.0; // 46
/// Distance from the screen edge to the near edge of the rail.
pub const EDGE_GAP: f32 = 10.0;
/// Gap between the rail and its content panel.
pub const RAIL_PANEL_GAP: f32 = 8.0;

// --- colours ------------------------------------------------------------
const RAIL_BG:     egui::Color32 = egui::Color32::from_rgb(0x17, 0x19, 0x1C);
const RAIL_STROKE: egui::Color32 = egui::Color32::from_rgb(0x2E, 0x31, 0x38);
const ICON_FG:     egui::Color32 = egui::Color32::from_rgb(0xB0, 0xB4, 0xBC);

// --- helpers ------------------------------------------------------------

/// Vertical rail anchored to a screen corner, identical shape on left and right.
pub fn icon_rail(
    id: &'static str,
    ctx: &egui::Context,
    anchor: egui::Align2,
    add_buttons: impl FnOnce(&mut egui::Ui),
) {
    // Signs for the offset: positive X pulls inward from the anchored edge.
    let offset = match anchor {
        egui::Align2::LEFT_TOP     => egui::vec2( EDGE_GAP,  EDGE_GAP),
        egui::Align2::RIGHT_TOP    => egui::vec2(-EDGE_GAP,  EDGE_GAP),
        egui::Align2::LEFT_BOTTOM  => egui::vec2( EDGE_GAP, -EDGE_GAP),
        egui::Align2::RIGHT_BOTTOM => egui::vec2(-EDGE_GAP, -EDGE_GAP),
        _ => egui::vec2(EDGE_GAP, EDGE_GAP),
    };

    let frame = egui::Frame {
        inner_margin: egui::Margin { left: 5, right: 5, top: 5, bottom: 5 },
        outer_margin: egui::Margin::ZERO,
        fill: RAIL_BG,
        stroke: egui::Stroke::new(1.0, RAIL_STROKE),
        corner_radius: egui::CornerRadius::same(10),
        shadow: egui::epaint::Shadow {
            offset: [0, 4], blur: 12, spread: 0,
            color: egui::Color32::from_black_alpha(120),
        },
    };

    egui::Area::new(egui::Id::new(id))
        .anchor(anchor, offset)
        .interactable(true)
        .show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    add_buttons(ui);
                });
            });
        });
}

/// One square letter button.
pub fn icon_button(
    ui: &mut egui::Ui,
    glyph: &str,
    tooltip: &str,
    is_active: bool,
    on_click: impl FnOnce(),
) {
    let color = if is_active { accent_color() } else { ICON_FG };
    // Monospace + fixed button size → W, S, I all render the same footprint.
    let text = egui::RichText::new(glyph)
        .size(16.0)
        .strong()
        .family(egui::FontFamily::Monospace)
        .color(color);
    let btn = egui::Button::new(text)
        .corner_radius(egui::CornerRadius::same(6))
        .selected(is_active);
    let resp = ui.add_sized([BTN, BTN], btn);
    if resp.on_hover_text(tooltip).clicked() {
        on_click();
    }
}

/// Floating content panel anchored to a screen corner with `RAIL_W + RAIL_PANEL_GAP`
/// of clearance so it never overlaps the rail.
pub fn floating_window(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    anchor: egui::Align2,
    size: egui::Vec2,
    open: &mut bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> Option<egui::Vec2> {
    let side_inset = EDGE_GAP + RAIL_W + RAIL_PANEL_GAP;
    let anchor_offset = match anchor {
        egui::Align2::LEFT_TOP     => egui::vec2( side_inset,  EDGE_GAP),
        egui::Align2::RIGHT_TOP    => egui::vec2(-side_inset,  EDGE_GAP),
        egui::Align2::LEFT_BOTTOM  => egui::vec2( side_inset, -EDGE_GAP),
        egui::Align2::RIGHT_BOTTOM => egui::vec2(-side_inset, -EDGE_GAP),
        _ => egui::vec2(side_inset, EDGE_GAP),
    };

    let frame = egui::Frame {
        inner_margin: egui::Margin { left: 12, right: 12, top: 8, bottom: 10 },
        outer_margin: egui::Margin::ZERO,
        fill: egui::Color32::from_rgb(0x1E, 0x20, 0x24),
        stroke: egui::Stroke::new(1.0, RAIL_STROKE),
        corner_radius: egui::CornerRadius::same(8),
        shadow: egui::epaint::Shadow {
            offset: [0, 6], blur: 18, spread: 0,
            color: egui::Color32::from_black_alpha(160),
        },
    };

    let response = egui::Window::new(title)
        .id(egui::Id::new(id))
        .title_bar(false)
        .resizable(true)
        .collapsible(false)
        .anchor(anchor, anchor_offset)
        .default_size(size)
        .frame(frame)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).heading().color(accent_color()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").on_hover_text("Close").clicked() {
                        *open = false;
                    }
                });
            });
            ui.separator();
            add_contents(ui);
        });

    response.map(|r| r.response.rect.size())
}
