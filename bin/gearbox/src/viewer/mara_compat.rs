//! Thin Gearbox-side compatibility layer over Mara.
//!
//! Gearbox's viewer UI still uses the early panel/ribbon helper shape.
//! Mara is the replacement crate, but its public API moved to
//! slot ribbons and typed UI surfaces. Keeping this shim local lets the
//! app drop the old UI crate while we migrate the panel code incrementally.

use std::hash::Hash;
use std::ops::RangeInclusive;

use bevy_egui::egui;

pub use bevy_mara::{
    AccentColor, CommandPaletteState, GlassOpacity, PaletteItem, RibbonCluster, RibbonDrag,
    RibbonEdge, RibbonGlyph, RibbonMode, RibbonOpen, RibbonPlacement, RibbonRole, RibbonWidth,
};

pub mod style {
    use bevy_egui::egui;

    pub const TEXT_PRIMARY: egui::Color32 = egui::Color32::from_rgb(0xE6, 0xE6, 0xE8);
    pub const TEXT_SECONDARY: egui::Color32 = egui::Color32::from_rgb(0x9A, 0x9A, 0xA2);
    pub const SUCCESS: egui::Color32 = egui::Color32::from_rgb(0x34, 0xC7, 0x59);
    pub const WARNING: egui::Color32 = egui::Color32::from_rgb(0xF5, 0xA5, 0x24);
    pub const DANGER: egui::Color32 = egui::Color32::from_rgb(0xEF, 0x44, 0x44);

    pub mod space {
        pub const TIGHT: f32 = 2.0;
        pub const BLOCK: f32 = 4.0;
    }

    pub fn srgb_to_egui(rgb: [f32; 3]) -> egui::Color32 {
        egui::Color32::from_rgb(
            (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        )
    }
}

pub const TREE_ROW_H: f32 = 20.0;
pub const TREE_INDENT: f32 = 14.0;

#[derive(Clone, Copy, Debug)]
pub struct RibbonDef {
    pub id: &'static str,
    pub edge: RibbonEdge,
    pub role: RibbonRole,
    pub mode: RibbonMode,
    pub draggable: bool,
    pub accepts: &'static [&'static str],
}

#[derive(Clone, Copy, Debug)]
pub struct RibbonItem {
    pub id: &'static str,
    pub ribbon: &'static str,
    pub cluster: RibbonCluster,
    pub slot: u32,
    pub glyph: RibbonGlyph,
    pub tooltip: &'static str,
    pub child_ribbon: Option<&'static str>,
    pub role: Option<RibbonRole>,
}

#[derive(Clone, Copy, Debug)]
pub struct RibbonClick {
    pub item: &'static str,
}

pub fn draw_assembly(
    ctx: &egui::Context,
    accent: impl Into<egui::Color32>,
    ribbons: &[RibbonDef],
    items: &[RibbonItem],
    open: &mut RibbonOpen,
    _placement: &mut RibbonPlacement,
    _drag: &mut RibbonDrag,
    is_active: impl Fn(&'static str) -> bool,
) -> Vec<RibbonClick> {
    let accent = accent.into();
    let mut clicks = Vec::new();
    for ribbon in ribbons {
        let anchor = match ribbon.edge {
            RibbonEdge::Left => egui::Align2::LEFT_TOP,
            RibbonEdge::Right => egui::Align2::RIGHT_TOP,
            RibbonEdge::Top => egui::Align2::LEFT_TOP,
            RibbonEdge::Bottom => egui::Align2::LEFT_BOTTOM,
        };
        let offset = match ribbon.edge {
            RibbonEdge::Left => egui::vec2(6.0, 48.0),
            RibbonEdge::Right => egui::vec2(-6.0, 48.0),
            RibbonEdge::Top => egui::vec2(48.0, 6.0),
            RibbonEdge::Bottom => egui::vec2(48.0, -6.0),
        };
        egui::Area::new(egui::Id::new(("gearbox_mara_ribbon", ribbon.id)))
            .anchor(anchor, offset)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let vertical = matches!(ribbon.edge, RibbonEdge::Left | RibbonEdge::Right);
                let mut ribbon_items: Vec<&RibbonItem> = items
                    .iter()
                    .filter(|item| item.ribbon == ribbon.id)
                    .collect();
                ribbon_items.sort_by_key(|item| (cluster_order(item.cluster), item.slot));
                let mut add_items = |ui: &mut egui::Ui, clicks: &mut Vec<RibbonClick>| {
                    for item in &ribbon_items {
                        let active = open.is_open(item.ribbon, item.id) || is_active(item.id);
                        let fill = if active {
                            accent
                        } else {
                            egui::Color32::from_rgba_unmultiplied(24, 28, 34, 210)
                        };
                        let stroke = if active {
                            egui::Stroke::new(1.0, accent)
                        } else {
                            egui::Stroke::new(1.0, egui::Color32::from_gray(56))
                        };
                        let button = egui::Button::new(glyph_label(item.glyph))
                            .min_size(egui::Vec2::splat(34.0))
                            .fill(fill)
                            .stroke(stroke);
                        let mut response = ui.add(button);
                        if !item.tooltip.is_empty() {
                            response = response.on_hover_text(item.tooltip);
                        }
                        if response.clicked() {
                            let role = item.role.unwrap_or(ribbon.role);
                            if role != RibbonRole::Icon {
                                open.toggle(item.ribbon, item.id);
                            }
                            clicks.push(RibbonClick { item: item.id });
                        }
                    }
                };
                if vertical {
                    ui.vertical(|ui| add_items(ui, &mut clicks));
                } else {
                    ui.horizontal(|ui| add_items(ui, &mut clicks));
                }
            });
    }
    clicks
}

fn cluster_order(cluster: RibbonCluster) -> u8 {
    match cluster {
        RibbonCluster::Start => 0,
        RibbonCluster::Middle => 1,
        RibbonCluster::End => 2,
    }
}

fn glyph_label(glyph: RibbonGlyph) -> &'static str {
    match glyph {
        RibbonGlyph::Text(s) | RibbonGlyph::Icon(s) => s,
        RibbonGlyph::Svg(_) => "◇",
    }
}

pub struct PaneBuilder<'a> {
    ui: &'a mut egui::Ui,
    accent: egui::Color32,
}

impl PaneBuilder<'_> {
    pub fn section(
        &mut self,
        _id: &'static str,
        title: &str,
        _default_open: bool,
        add: impl FnOnce(&mut egui::Ui),
    ) {
        let accent = self.accent;
        egui::Frame::group(self.ui.style())
            .fill(egui::Color32::from_rgba_unmultiplied(16, 18, 24, 218))
            .stroke(egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 120),
            ))
            .inner_margin(egui::Margin::same(8))
            .show(self.ui, |ui| {
                ui.label(egui::RichText::new(title).small().strong().color(accent));
                ui.add_space(style::space::TIGHT);
                add(ui);
            });
        self.ui.add_space(style::space::BLOCK);
    }
}

pub fn floating_window_for_item(
    ctx: &egui::Context,
    _ribbons: &[RibbonDef],
    _items: &[RibbonItem],
    _placement: &RibbonPlacement,
    item: &'static str,
    title: &'static str,
    size: egui::Vec2,
    keep: &mut bool,
    accent: impl Into<egui::Color32>,
    add: impl FnOnce(&mut PaneBuilder<'_>),
) {
    let accent = accent.into();
    egui::Window::new(title)
        .id(egui::Id::new(("gearbox_mara_panel", item)))
        .default_size(size)
        .min_width(size.x.min(220.0))
        .open(keep)
        .show(ctx, |ui| {
            let mut pane = PaneBuilder { ui, accent };
            add(&mut pane);
        });
}

pub fn sub_caption(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .small()
            .color(style::TEXT_SECONDARY),
    );
}

pub fn search_field(
    ui: &mut egui::Ui,
    value: &mut String,
    hint: &str,
    _accent: impl Into<egui::Color32>,
) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), 24.0],
        egui::TextEdit::singleline(value).hint_text(hint),
    )
}

pub fn readout_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.set_min_height(20.0);
        ui.add_sized(
            [92.0, 18.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .small()
                    .color(style::TEXT_SECONDARY),
            ),
        );
        ui.label(
            egui::RichText::new(value)
                .small()
                .color(style::TEXT_PRIMARY),
        );
    });
}

pub fn keybinding_row(ui: &mut egui::Ui, keys: &str, action: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [82.0, 20.0],
            egui::Label::new(
                egui::RichText::new(keys)
                    .monospace()
                    .small()
                    .color(style::SUCCESS),
            ),
        );
        ui.label(
            egui::RichText::new(action)
                .small()
                .color(style::TEXT_PRIMARY),
        );
    });
}

pub fn wide_button(
    ui: &mut egui::Ui,
    label: &str,
    accent: impl Into<egui::Color32>,
) -> egui::Response {
    let accent = accent.into();
    ui.add_sized(
        [ui.available_width(), 28.0],
        egui::Button::new(label).fill(egui::Color32::from_rgba_unmultiplied(
            accent.r(),
            accent.g(),
            accent.b(),
            38,
        )),
    )
}

pub fn chip(ui: &mut egui::Ui, label: &str, accent: impl Into<egui::Color32>) {
    let accent = accent.into();
    chip_colored(ui, label, accent, accent);
}

pub fn chip_colored(
    ui: &mut egui::Ui,
    label: &str,
    color: impl Into<egui::Color32>,
    _accent: impl Into<egui::Color32>,
) {
    let color = color.into();
    ui.label(
        egui::RichText::new(format!(" {label} "))
            .small()
            .background_color(egui::Color32::from_rgba_unmultiplied(
                color.r(),
                color.g(),
                color.b(),
                44,
            ))
            .color(color),
    );
}

pub fn badge_row(
    ui: &mut egui::Ui,
    label: &str,
    values: &[&str],
    accent: impl Into<egui::Color32>,
) {
    let accent = accent.into();
    ui.horizontal_wrapped(|ui| {
        ui.add_sized(
            [82.0, 18.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .small()
                    .color(style::TEXT_SECONDARY),
            ),
        );
        for value in values {
            chip(ui, value, accent);
        }
    });
}

pub fn toggle(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut bool,
    _accent: impl Into<egui::Color32>,
) -> egui::Response {
    ui.checkbox(value, label)
}

pub fn pretty_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: RangeInclusive<f64>,
    decimals: usize,
    suffix: &str,
    _accent: impl Into<egui::Color32>,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized(
            [110.0, 18.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .small()
                    .color(style::TEXT_SECONDARY),
            ),
        );
        ui.add(
            egui::Slider::new(value, range)
                .max_decimals(decimals)
                .suffix(suffix),
        )
    })
    .inner
}

pub fn row_separator(ui: &mut egui::Ui) {
    ui.separator();
}

#[derive(Clone, Copy, Debug)]
pub enum TreeIconKind {
    Eye,
    Color(egui::Color32),
    Glyph { on: &'static str, off: &'static str },
}

pub struct TreeIconSlot<'a> {
    kind: TreeIconKind,
    value: &'a mut bool,
    tooltip: Option<&'static str>,
}

impl<'a> TreeIconSlot<'a> {
    pub fn new(kind: TreeIconKind, value: &'a mut bool) -> Self {
        Self {
            kind,
            value,
            tooltip: None,
        }
    }

    pub fn with_tooltip(mut self, tooltip: &'static str) -> Self {
        self.tooltip = Some(tooltip);
        self
    }
}

pub struct TreeRowResponse {
    pub body: egui::Response,
    pub icons: Vec<egui::Response>,
}

pub fn tree_row(
    ui: &mut egui::Ui,
    id_salt: impl Hash,
    depth: u32,
    mut open: Option<&mut bool>,
    _leading: Option<TreeIconKind>,
    label: &str,
    selected: bool,
    accent: impl Into<egui::Color32>,
    slots: &mut [TreeIconSlot<'_>],
) -> TreeRowResponse {
    let accent = accent.into();
    ui.push_id(id_salt, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * TREE_INDENT);
            if let Some(open) = open.as_deref_mut() {
                let glyph = if *open { "▾" } else { "▸" };
                if ui.small_button(glyph).clicked() {
                    *open = !*open;
                }
            } else {
                ui.add_space(22.0);
            }
            let text = if selected {
                egui::RichText::new(label).color(accent).strong()
            } else {
                egui::RichText::new(label).color(style::TEXT_PRIMARY)
            };
            let body = ui.selectable_label(selected, text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut icons = Vec::with_capacity(slots.len());
                for slot in slots.iter_mut().rev() {
                    let response = draw_tree_icon(ui, slot);
                    icons.push(response);
                }
                icons.reverse();
                TreeRowResponse { body, icons }
            })
            .inner
        })
        .inner
    })
    .inner
}

fn draw_tree_icon(ui: &mut egui::Ui, slot: &mut TreeIconSlot<'_>) -> egui::Response {
    let (label, fill) = match slot.kind {
        TreeIconKind::Eye => {
            if *slot.value {
                ("👁", egui::Color32::TRANSPARENT)
            } else {
                ("–", egui::Color32::TRANSPARENT)
            }
        }
        TreeIconKind::Color(color) => ("■", color),
        TreeIconKind::Glyph { on, off } => {
            if *slot.value {
                (on, egui::Color32::TRANSPARENT)
            } else {
                (off, egui::Color32::TRANSPARENT)
            }
        }
    };
    let mut button = egui::Button::new(label).min_size(egui::vec2(22.0, 20.0));
    if fill != egui::Color32::TRANSPARENT {
        button = button.fill(fill);
    }
    let mut response = ui.add(button);
    if let Some(tooltip) = slot.tooltip {
        response = response.on_hover_text(tooltip);
    }
    if response.clicked() {
        *slot.value = !*slot.value;
    }
    response
}

pub fn context_menu_mara(
    response: &egui::Response,
    _accent: impl Into<egui::Color32>,
    add: impl FnOnce(&mut egui::Ui),
) {
    response.context_menu(add);
}

pub struct HybridSelectResponse {
    pub body: egui::Response,
    pub radio: egui::Response,
}

pub fn hybrid_select_row(
    ui: &mut egui::Ui,
    id_salt: impl Hash,
    label: &str,
    trailing: Option<&str>,
    selected: bool,
    radio_selected: bool,
    accent: impl Into<egui::Color32>,
) -> HybridSelectResponse {
    let accent = accent.into();
    ui.push_id(id_salt, |ui| {
        ui.horizontal(|ui| {
            let radio = ui.add(egui::Button::new(if radio_selected {
                "●"
            } else {
                "○"
            }));
            let text = if selected {
                egui::RichText::new(label).strong().color(accent)
            } else {
                egui::RichText::new(label).color(style::TEXT_PRIMARY)
            };
            let body = ui.selectable_label(selected, text);
            if let Some(trailing) = trailing {
                ui.label(
                    egui::RichText::new(trailing)
                        .small()
                        .color(style::TEXT_SECONDARY),
                );
            }
            HybridSelectResponse { body, radio }
        })
        .inner
    })
    .inner
}

pub fn command_palette(
    ctx: &egui::Context,
    state: &mut CommandPaletteState,
    items: &[PaletteItem],
    accent: impl Into<egui::Color32>,
) -> Option<&'static str> {
    if !state.open {
        return None;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.open = false;
        return None;
    }

    let accent = accent.into();
    let query = state.query.to_lowercase();
    let mut picked = None;
    egui::Window::new("Command palette")
        .id(egui::Id::new("gearbox_mara_command_palette"))
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .show(ctx, |ui| {
            ui.add_sized(
                [ui.available_width(), 26.0],
                egui::TextEdit::singleline(&mut state.query).hint_text("Search commands…"),
            );
            ui.add_space(style::space::BLOCK);
            for item in items
                .iter()
                .filter(|item| query.is_empty() || item.label.to_lowercase().contains(&query))
            {
                let label = match item.hint {
                    Some(hint) => format!("{}    {}", item.label, hint),
                    None => item.label.to_owned(),
                };
                if ui
                    .add_sized(
                        [ui.available_width(), 24.0],
                        egui::Button::new(label).fill(egui::Color32::from_rgba_unmultiplied(
                            accent.r(),
                            accent.g(),
                            accent.b(),
                            24,
                        )),
                    )
                    .clicked()
                {
                    picked = Some(item.id);
                }
            }
        });
    if picked.is_some() {
        state.open = false;
    }
    picked
}
