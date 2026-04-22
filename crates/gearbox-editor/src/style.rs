//! One-shot egui theme setup.
//!
//! Palette + typography follow the 2024-2026 editor convergence
//! (Blender 4, UE5.4, Godot 4, Unity 6, Fleet). All values are
//! centralised here so individual panels never hard-code colours —
//! the full palette is published even if not every token has a
//! current caller, so new UI pulls from the same reference set.
//!
//! The "accent" is dynamic: when a vehicle is selected, we swap the
//! accent to the vehicle's chassis colour — green tractor → green
//! menus, yellow harvester → yellow menus. The theme is re-applied
//! whenever the resource changes; panels read the resource directly
//! for per-widget tints (section headers, progress-bar fills, etc).

#![allow(dead_code)]

use bevy::prelude::*;
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

/// Live accent colour — swapped in from the selected vehicle's chassis
/// colour each frame. Defaults to white when nothing's selected (neutral
/// readout); becomes the vehicle's chassis colour on selection.
#[derive(Resource, Copy, Clone, Debug, PartialEq, Eq)]
pub struct AccentColor(pub egui::Color32);

/// Neutral accent used when no vehicle is selected.
pub const ACCENT_NEUTRAL: egui::Color32 = egui::Color32::from_rgb(0xE6, 0xE6, 0xE8);

impl Default for AccentColor {
    fn default() -> Self { Self(ACCENT_NEUTRAL) }
}

// ─── Embedded UI font ───────────────────────────────────────────────
//
// Iosevka Term Light baked into the binary via `include_bytes!` — no
// `assets/` directory needs to ship alongside the executable. Face 0
// of the upstream `SGr-IosevkaTerm-Light.ttc`, subset to Latin +
// common symbol blocks (~1.3 MB).
//
// We deliberately stick with the stock egui font families
// (`Proportional` + `Monospace`) and do NOT register `FontFamily::Name`
// variants: `ctx.set_fonts` only takes effect on the NEXT `begin_pass`,
// and bevy_egui 0.39 spawns the primary egui context entity late
// enough that we can't race ahead of frame 0's draw. Looking up an
// unbound `FontFamily::Name("…")` on frame 0 is a hard panic in
// epaint, so we give up per-text weight selection and use size +
// colour + `.strong()` for hierarchy instead.

const IOSEVKA_LIGHT_TTF: &[u8] = include_bytes!("fonts/iosevka-light.ttf");

/// Replace egui's stock body fonts with Iosevka Light. Only touches
/// `Proportional` + `Monospace` — both families exist by default, so
/// even if `set_fonts` doesn't apply until frame 1, frame-0 draws
/// render with egui's built-ins and don't panic.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "iosevka-light".into(),
        std::sync::Arc::new(egui::FontData::from_static(IOSEVKA_LIGHT_TTF)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "iosevka-light".into());
    }
    ctx.set_fonts(fonts);
}

/// Piggyback on `apply_theme`'s `Local` state: run once, the first
/// time the system fires with a valid context. Works because UI
/// systems only ever reference stock font families, so even the
/// one-frame delay between `ctx.set_fonts` and its effect is
/// harmless.
fn install_fonts_once(ctx: &egui::Context, installed: &mut bool) {
    if *installed { return; }
    install_fonts(ctx);
    *installed = true;
}

/// Re-apply the egui theme when the `AccentColor` resource changes.
/// `last_applied` is stored per-system so we only push a new style
/// when the colour actually differs — `ctx.set_style` blows the egui
/// style cache, so avoid doing it every frame. Font install also
/// rides on this system, gated by `fonts_installed` — runs once on
/// the first successful tick.
pub fn apply_theme(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut last_applied: Local<Option<egui::Color32>>,
    mut fonts_installed: Local<bool>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    install_fonts_once(ctx, &mut fonts_installed);

    if *last_applied == Some(accent.0) { return }
    let accent_col = accent.0;

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill          = BG_1_PANEL;
    visuals.window_fill         = BG_1_PANEL;
    visuals.window_stroke       = egui::Stroke::new(1.0, BORDER_SUBTLE);
    visuals.extreme_bg_color    = BG_4_INPUT;
    visuals.faint_bg_color      = BG_2_RAISED;
    visuals.code_bg_color       = BG_4_INPUT;
    visuals.override_text_color = Some(TEXT_PRIMARY);
    visuals.selection.bg_fill   = tinted_surface(accent_col);
    visuals.selection.stroke    = egui::Stroke::new(1.0, accent_col);
    visuals.hyperlink_color     = accent_col;

    // Slightly larger rounding — matches modern editor tools (Fleet,
    // Zed, Helix) and reads less sharp-edged than the old 4 px.
    let r = egui::CornerRadius::same(6);
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
    visuals.widgets.active         = widget(accent_col,  TEXT_PRIMARY,   accent_col);
    visuals.widgets.open           = widget(BG_3_HOVER,  TEXT_PRIMARY,   BORDER_INNER);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals;

    // Slightly roomier controls — interacts at 20 px (was 18) and
    // buttons get 8×4 padding (was 6×2) so rows don't feel cramped
    // against each other.
    style.spacing.item_spacing      = egui::vec2(6.0, 3.0);
    style.spacing.button_padding    = egui::vec2(8.0, 4.0);
    style.spacing.indent            = 14.0;
    style.spacing.window_margin     = egui::Margin::ZERO;
    style.spacing.interact_size.y   = 20.0;
    // Tight slider track. Combined with no inline `.text(...)` label
    // and no `.show_value()` suffix, this leaves enough right-cell
    // space for the slider PLUS the current value without pushing
    // the section card wider than its pinned inner width.
    style.spacing.slider_width      = 90.0;
    style.spacing.icon_width        = 14.0;
    style.spacing.icon_spacing      = 6.0;
    style.text_styles = [
        (egui::TextStyle::Heading,   egui::FontId::new(16.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Body,      egui::FontId::new(13.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Monospace, egui::FontId::new(13.0, egui::FontFamily::Monospace)),
        (egui::TextStyle::Button,    egui::FontId::new(13.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Small,     egui::FontId::new(12.0, egui::FontFamily::Proportional)),
    ]
    .into();

    ctx.set_style(style);
    *last_applied = Some(accent_col);
}

/// Update the accent colour from the currently selected vehicle's
/// chassis colour. Runs before panels so the new colour lands in the
/// same frame the selection changes.
pub fn update_accent_from_selection(
    selection: Res<super::selection::Selection>,
    sim: Res<gearbox_viz::GearboxSim>,
    mut accent: ResMut<AccentColor>,
) {
    let new_color = selection
        .vehicle
        .and_then(|id| sim.0.vehicle(id))
        .map(|v| srgb_to_egui(v.spec.chassis.color))
        .unwrap_or(ACCENT_NEUTRAL);
    if accent.0 != new_color {
        accent.0 = new_color;
    }
}

/// Darker/muted version of an accent colour — used for "selected" row
/// fills where the full-strength accent would be too loud.
fn tinted_surface(c: egui::Color32) -> egui::Color32 {
    // 35 % of accent over BG_2_RAISED.
    let f = 0.35;
    let lerp = |a: u8, b: u8| ((a as f32) * (1.0 - f) + (b as f32) * f).round() as u8;
    egui::Color32::from_rgb(
        lerp(BG_2_RAISED.r(), c.r()),
        lerp(BG_2_RAISED.g(), c.g()),
        lerp(BG_2_RAISED.b(), c.b()),
    )
}

/// Convert the linear-sRGB `[f32;3]` we store on `ChassisSpec` to an
/// egui Color32. Matches the visual tone the 3D view renders.
fn srgb_to_egui(rgb: [f32; 3]) -> egui::Color32 {
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgb(to_u8(rgb[0]), to_u8(rgb[1]), to_u8(rgb[2]))
}

/// Uppercase accent section header. Used both by left panels
/// (`CollapsingHeader::new(section_caps(…))`) and by the right
/// inspector — keeps the visual language identical on both sides.
///
/// Pops against the Light body font via `.strong()` (darker render)
/// + caps + accent colour. Per-weight font selection isn't available:
/// see the comment on the embedded-font block above for why.
///
/// Size: 12 pt body baseline + 15 % bump so section titles read
/// clearly larger than body copy inside the same card.
pub fn section_caps(label: &str, accent: egui::Color32) -> egui::RichText {
    egui::RichText::new(label.to_uppercase())
        .strong()
        .size(12.0 * 1.15)
        .color(accent)
}

pub fn fg_dim() -> egui::Color32 { TEXT_SECONDARY }

// ─── Design-system tokens ────────────────────────────────────────────
//
// Every panel should lay out against THESE instead of ad-hoc `add_space`
// calls. Keeps rhythm consistent and lets the whole UI be re-tuned
// from one place. Scale is a 4 px grid; sizes are named by use, not
// by pixel count, so the numbers can evolve without a find-and-replace.

pub mod space {
    /// Between tightly-related items inside one row (label↔chip, glyph↔text).
    pub const TIGHT: f32 = 2.0;
    /// Between adjacent rows inside one section (label rows, slider rows).
    pub const ROW: f32 = 2.0;
    /// Between a row and a sub-block inside one section.
    pub const BLOCK: f32 = 4.0;
    /// Between distinct section cards in a panel. Slight gap so the
    /// rounded frames don't kiss each other edge-to-edge.
    pub const SECTION: f32 = 3.0;
}

pub mod radius {
    /// Progress bars, chips, bars-within-rows.
    pub const SM: u8 = 3;
    /// Buttons, cards.
    pub const MD: u8 = 6;
    /// Panels, pop-overs.
    pub const LG: u8 = 8;
}

pub mod font {
    //! Typographic hierarchy — specific sizes so "small", "body",
    //! "strong" read as distinct tiers. Bodies 11 pt; captions 10;
    //! small-numeric (monospace, readouts) 11.
    pub const TITLE: f32 = 13.0;
    pub const BODY: f32 = 11.0;
    pub const CAPTION: f32 = 10.0;
    pub const NUMERIC: f32 = 11.0;
}

/// Draw a 1 px subtle divider line across the current row. Used to
/// separate the section header from its body and to split in-section
/// blocks (e.g. vehicle info vs controls).
pub fn divider(ui: &mut egui::Ui) {
    let full_width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(full_width, 1.0),
        egui::Sense::empty(),
    );
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        egui::Stroke::new(1.0, BORDER_SUBTLE),
    );
}

/// Uppercase title text for panel-level headings (above sections).
/// Pops against the Light body font via `.strong()` + an enlarged
/// point size (20 % above `font::TITLE`) + primary text colour.
pub fn title_text(label: &str) -> egui::RichText {
    egui::RichText::new(label)
        .strong()
        .size(font::TITLE * 1.20)
        .color(TEXT_PRIMARY)
}

/// Small dim label — the "what is this row" caption-sized text that
/// sits in the left cell of a labelled row. Uses egui's `.small()`
/// text style so the compact panel density is preserved.
pub fn body_label(label: &str) -> egui::RichText {
    egui::RichText::new(label).small().color(TEXT_SECONDARY)
}

/// Italic caption — for under-row hints ("drag to edit", etc.).
pub fn caption(label: &str) -> egui::RichText {
    egui::RichText::new(label).small().italics().color(TEXT_DISABLED)
}

/// Text colour that stays readable on top of an arbitrary accent
/// fill. Uses Rec. 709 luma of the fill — bright fills get near-black
/// text, dim fills get white — so progress-bar readouts never
/// disappear into the accent when the user drives a yellow harvester
/// or a pastel-lavender husky.
pub fn contrast_text_for(fill: egui::Color32) -> egui::Color32 {
    let r = fill.r() as f32 / 255.0;
    let g = fill.g() as f32 / 255.0;
    let b = fill.b() as f32 / 255.0;
    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if luma > 0.55 {
        egui::Color32::from_rgb(0x18, 0x18, 0x1C)
    } else {
        TEXT_PRIMARY
    }
}
