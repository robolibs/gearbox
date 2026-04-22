//! Reusable egui widgets shared across editor panels.
//!
//! Keep these purely presentational — they should not know about
//! `Selection`, `PendingSpawn`, or any domain state. Pass plain values
//! (strings, colours, closures) in and let the caller react to the
//! returned `Response`.

use bevy_egui::egui;

use super::style::{fg_dim, TEXT_PRIMARY};

/// A full-width "card" button — accent glyph on the left, primary
/// name + small subtitle on the right. Reads like UE5's "Create"
/// entries. Used for preset spawn buttons, but generic enough for
/// any "pick an item" list.
pub fn card_button(
    ui: &mut egui::Ui,
    glyph: &str,
    name: &str,
    subtitle: &str,
    accent: egui::Color32,
) -> egui::Response {
    const ROW_H: f32 = 32.0;
    let w = ui.available_width();
    let btn = egui::Button::new("")
        .corner_radius(egui::CornerRadius::same(6))
        .min_size(egui::vec2(w, ROW_H));
    let resp = ui.add_sized([w, ROW_H], btn);

    // Custom-paint inside the button's rect so we can stack two
    // differently-sized strings side-by-side with a coloured glyph.
    let rect = resp.rect;
    let painter = ui.painter_at(rect);
    let text_rect = rect.shrink2(egui::vec2(8.0, 0.0));

    painter.text(
        egui::pos2(text_rect.min.x, text_rect.center().y),
        egui::Align2::LEFT_CENTER,
        glyph,
        egui::FontId::proportional(14.0),
        accent,
    );
    painter.text(
        egui::pos2(text_rect.min.x + 22.0, text_rect.center().y - 6.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(12.0),
        TEXT_PRIMARY,
    );
    painter.text(
        egui::pos2(text_rect.min.x + 22.0, text_rect.center().y + 7.0),
        egui::Align2::LEFT_CENTER,
        subtitle,
        egui::FontId::proportional(10.0),
        fg_dim(),
    );
    resp
}

/// A key-chip + label row used in the "Keys" help section.
pub fn keybinding_row(ui: &mut egui::Ui, keys: &str, action: &str) {
    ui.horizontal(|ui| {
        let chip = egui::RichText::new(keys)
            .monospace()
            .small()
            .color(ui.visuals().text_color());
        let frame = egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .inner_margin(egui::Margin::symmetric(5, 1))
            .corner_radius(egui::CornerRadius::same(3));
        frame.show(ui, |ui| ui.label(chip));
        ui.add_space(6.0);
        ui.label(egui::RichText::new(action).small().color(fg_dim()));
    });
}
