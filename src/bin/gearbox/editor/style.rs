//! One-shot egui theme setup — dark, generous padding, rounded corners,
//! a subtle accent.  Applied once on the first frame.

use bevy::prelude::Local;
use bevy_egui::{egui, EguiContexts};

const ACCENT:   egui::Color32 = egui::Color32::from_rgb(0xE8, 0x8B, 0x28); // amber
const BG_DEEP:  egui::Color32 = egui::Color32::from_rgb(0x14, 0x15, 0x17);
const BG_PANEL: egui::Color32 = egui::Color32::from_rgb(0x1E, 0x20, 0x24);
const BG_RAISE: egui::Color32 = egui::Color32::from_rgb(0x2A, 0x2D, 0x33);
const BG_HOVER: egui::Color32 = egui::Color32::from_rgb(0x38, 0x3C, 0x44);
const FG_DIM:   egui::Color32 = egui::Color32::from_rgb(0x8B, 0x90, 0x9B);
const FG_TEXT:  egui::Color32 = egui::Color32::from_rgb(0xD7, 0xDA, 0xE0);
const STROKE:   egui::Color32 = egui::Color32::from_rgb(0x35, 0x38, 0x3F);

pub fn apply_theme_once(mut contexts: EguiContexts, mut applied: Local<bool>) {
    if *applied { return }
    let Ok(ctx) = contexts.ctx_mut() else { return };

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill       = BG_PANEL;
    visuals.window_fill      = BG_PANEL;
    visuals.window_stroke    = egui::Stroke::new(1.0, STROKE);
    visuals.extreme_bg_color = BG_DEEP;
    visuals.faint_bg_color   = BG_RAISE;
    visuals.code_bg_color    = BG_DEEP;
    visuals.override_text_color = Some(FG_TEXT);
    visuals.selection.bg_fill = ACCENT.linear_multiply(0.4);
    visuals.selection.stroke  = egui::Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color   = ACCENT;

    let r = egui::CornerRadius::same(5);
    let widget = |bg, fg_stroke, bg_stroke| egui::style::WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: bg,
        bg_stroke: egui::Stroke::new(1.0, bg_stroke),
        fg_stroke: egui::Stroke::new(1.0, fg_stroke),
        corner_radius: r,
        expansion: 0.0,
    };
    visuals.widgets.noninteractive = widget(BG_PANEL, FG_DIM,  STROKE);
    visuals.widgets.inactive       = widget(BG_RAISE, FG_TEXT, STROKE);
    visuals.widgets.hovered        = widget(BG_HOVER, FG_TEXT, ACCENT);
    visuals.widgets.active         = widget(ACCENT,   BG_DEEP, ACCENT);
    visuals.widgets.open           = widget(BG_HOVER, FG_TEXT, STROKE);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals;
    style.spacing.item_spacing      = egui::vec2(8.0, 8.0);
    style.spacing.button_padding    = egui::vec2(14.0, 7.0);
    style.spacing.indent            = 14.0;
    style.spacing.window_margin     = egui::Margin::same(10);
    style.spacing.interact_size.y   = 26.0;
    style.text_styles = [
        (egui::TextStyle::Heading,     egui::FontId::new(16.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Body,        egui::FontId::new(13.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Monospace,   egui::FontId::new(12.0, egui::FontFamily::Monospace)),
        (egui::TextStyle::Button,      egui::FontId::new(13.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Small,       egui::FontId::new(11.0, egui::FontFamily::Proportional)),
    ]
    .into();

    ctx.set_style(style);
    *applied = true;
}

/// Small helper: section heading with a thin accent bar underneath.
pub fn section_header(ui: &mut egui::Ui, label: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(label.to_uppercase())
            .color(FG_DIM)
            .small()
            .strong(),
    );
    ui.separator();
}

pub fn accent_color() -> egui::Color32 { ACCENT }
pub fn fg_dim()       -> egui::Color32 { FG_DIM }
