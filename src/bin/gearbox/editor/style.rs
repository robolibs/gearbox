//! One-shot egui theme setup.
//!
//! Palette + typography follow the 2024-2026 editor convergence
//! (Blender 4, UE5.4, Godot 4, Unity 6, Fleet). All values are
//! centralised here so individual panels never hard-code colours.

use bevy::prelude::Local;
use bevy_egui::{egui, EguiContexts};

// ─── Neutrals ───────────────────────────────────────────────────────
pub const BG_0_WINDOW: egui::Color32 = egui::Color32::from_rgb(0x1A, 0x1A, 0x1C);
pub const BG_1_PANEL:  egui::Color32 = egui::Color32::from_rgb(0x24, 0x24, 0x28);
pub const BG_2_RAISED: egui::Color32 = egui::Color32::from_rgb(0x2D, 0x2D, 0x32);
pub const BG_3_HOVER:  egui::Color32 = egui::Color32::from_rgb(0x38, 0x38, 0x3F);
pub const BG_4_INPUT:  egui::Color32 = egui::Color32::from_rgb(0x18, 0x18, 0x1A);

pub const BORDER_SUBTLE: egui::Color32 = egui::Color32::from_rgb(0x0E, 0x0E, 0x10);
pub const BORDER_INNER:  egui::Color32 = egui::Color32::from_rgb(0x3A, 0x3A, 0x42);

// ─── Text ───────────────────────────────────────────────────────────
pub const TEXT_PRIMARY:   egui::Color32 = egui::Color32::from_rgb(0xE6, 0xE6, 0xE8);
pub const TEXT_SECONDARY: egui::Color32 = egui::Color32::from_rgb(0x9A, 0x9A, 0xA2);
pub const TEXT_DISABLED:  egui::Color32 = egui::Color32::from_rgb(0x5A, 0x5A, 0x62);

// ─── Accent (selection / focus) — violet / purple ──────────────────
pub const ACCENT:         egui::Color32 = egui::Color32::from_rgb(0xA7, 0x8B, 0xFA);
pub const ACCENT_HOVER:   egui::Color32 = egui::Color32::from_rgb(0xC4, 0xB5, 0xFD);
pub const ACCENT_PRESSED: egui::Color32 = egui::Color32::from_rgb(0x8B, 0x5C, 0xF6);
/// Subtle purple-tinted surface for the active side button and the
/// selected outliner row. 18 % of `ACCENT` over `BG_2_RAISED`.
pub const ACCENT_TINT:    egui::Color32 = egui::Color32::from_rgb(0x42, 0x3A, 0x5A);
pub const SELECTION_ROW:  egui::Color32 = egui::Color32::from_rgb(0x4A, 0x3C, 0x72);

// ─── Axes (vivid: gizmos + inspector labels) ────────────────────────
pub const AXIS_X: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x43, 0x3B);
pub const AXIS_Y: egui::Color32 = egui::Color32::from_rgb(0x7F, 0xB4, 0x35);
pub const AXIS_Z: egui::Color32 = egui::Color32::from_rgb(0x2E, 0x83, 0xE6);

// ─── Status ─────────────────────────────────────────────────────────
pub const SUCCESS: egui::Color32 = egui::Color32::from_rgb(0x34, 0xC7, 0x59);
pub const WARNING: egui::Color32 = egui::Color32::from_rgb(0xF5, 0xA5, 0x24);
pub const DANGER:  egui::Color32 = egui::Color32::from_rgb(0xEF, 0x44, 0x44);

pub fn apply_theme_once(mut contexts: EguiContexts, mut applied: Local<bool>) {
    if *applied { return }
    let Ok(ctx) = contexts.ctx_mut() else { return };

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill          = BG_1_PANEL;
    visuals.window_fill         = BG_1_PANEL;
    visuals.window_stroke       = egui::Stroke::new(1.0, BORDER_SUBTLE);
    visuals.extreme_bg_color    = BG_4_INPUT;
    visuals.faint_bg_color      = BG_2_RAISED;
    visuals.code_bg_color       = BG_4_INPUT;
    visuals.override_text_color = Some(TEXT_PRIMARY);
    visuals.selection.bg_fill   = SELECTION_ROW;
    visuals.selection.stroke    = egui::Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color     = ACCENT;

    let r = egui::CornerRadius::same(4);
    let widget = |bg: egui::Color32, fg_stroke: egui::Color32, bg_stroke: egui::Color32| {
        egui::style::WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: bg,
            bg_stroke: egui::Stroke::new(1.0, bg_stroke),
            fg_stroke: egui::Stroke::new(1.0, fg_stroke),
            corner_radius: r,
            expansion: 0.0,
        }
    };
    visuals.widgets.noninteractive = widget(BG_1_PANEL, TEXT_SECONDARY, BORDER_SUBTLE);
    visuals.widgets.inactive       = widget(BG_2_RAISED, TEXT_PRIMARY,   BORDER_SUBTLE);
    visuals.widgets.hovered        = widget(BG_3_HOVER,  TEXT_PRIMARY,   BORDER_INNER);
    visuals.widgets.active         = widget(ACCENT,      TEXT_PRIMARY,   ACCENT);
    visuals.widgets.open           = widget(BG_3_HOVER,  TEXT_PRIMARY,   BORDER_INNER);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals;

    // Spec density (2024-26 convergence):
    //   row height   22 px inspector / 20 px outliner
    //   item gap      6 × 3 px
    //   button pad    6 × 2 px
    //   indent       12 px
    style.spacing.item_spacing      = egui::vec2(6.0, 2.0);
    style.spacing.button_padding    = egui::vec2(6.0, 2.0);
    style.spacing.indent            = 12.0;
    style.spacing.window_margin     = egui::Margin::ZERO; // our Frame handles padding
    style.spacing.interact_size.y   = 18.0;
    style.spacing.slider_width      = 100.0;
    style.text_styles = [
        (egui::TextStyle::Heading,   egui::FontId::new(16.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Body,      egui::FontId::new(13.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Monospace, egui::FontId::new(13.0, egui::FontFamily::Monospace)),
        (egui::TextStyle::Button,    egui::FontId::new(13.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Small,     egui::FontId::new(12.0, egui::FontFamily::Proportional)),
    ]
    .into();

    ctx.set_style(style);
    *applied = true;
}

/// Uppercase accent section header. Used both by left panels
/// (`CollapsingHeader::new(section_caps(…))`) and by the right
/// inspector — keeps the visual language identical on both sides.
pub fn section_caps(label: &str) -> egui::RichText {
    egui::RichText::new(label.to_uppercase())
        .strong()
        .size(12.0)
        .color(ACCENT)
}

pub fn accent_color() -> egui::Color32 { ACCENT }
pub fn fg_dim()       -> egui::Color32 { TEXT_SECONDARY }
