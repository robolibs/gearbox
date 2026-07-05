//! Gearbox viewer UI helpers backed by Mara.
//!
//! The ribbon rail and command palette below go through Mara's real
//! renderers. The panel bodies still accept raw `egui::Ui` closures
//! because the legacy Gearbox panels have not yet been converted to
//! typed Mara `PaneBody`/`Pod` content.

use std::hash::Hash;
use std::ops::RangeInclusive;

use bevy_egui::egui;
use mara::prelude::MaraHostCtx;

pub use bevy_mara::pane::{Pane, PaneAnchor, PaneBody, PaneResize, RailZone};
pub use bevy_mara::pod::{Pod, PodResponse};
pub use bevy_mara::vocab::Id as MaraId;
pub use bevy_mara::{
    AccentColor, CommandPaletteState, GlassOpacity, PaletteItem, ResolvedSlotRibbon, RibbonAction,
    RibbonCluster, RibbonDrag, RibbonEdge, RibbonGlyph, RibbonMode, RibbonOpen, RibbonPlacement,
    RibbonRole, RibbonScope, RibbonSlotItem, RibbonWidth,
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
    placement: &mut RibbonPlacement,
    drag: &mut RibbonDrag,
    is_active: impl Fn(&'static str) -> bool,
) -> Vec<RibbonClick> {
    let accent: egui::Color32 = accent.into();
    let host = MaraHostCtx::ui_only(ctx, None);
    host.publish_ribbon_pane_ids(
        items
            .iter()
            .filter(|item| item.role.unwrap_or(RibbonRole::Panel) == RibbonRole::Panel)
            .map(|item| bevy_mara::vocab::Id::new(item.id)),
    );

    let mut resolved = Vec::new();
    for ribbon in ribbons {
        for cluster in [
            RibbonCluster::Start,
            RibbonCluster::Middle,
            RibbonCluster::End,
        ] {
            let mut ribbon_items: Vec<&RibbonItem> = items
                .iter()
                .filter(|item| item.ribbon == ribbon.id && item.cluster == cluster)
                .collect();
            ribbon_items.sort_by_key(|item| item.slot);
            let mut slot_items: Vec<RibbonSlotItem> = ribbon_items
                .into_iter()
                .map(|item| {
                    let mut slot_item = RibbonSlotItem::featureful(
                        item.id,
                        glyph_icon_payload(item.glyph),
                        item.id,
                        item.tooltip,
                        RibbonAction::Command(bevy_mara::vocab::Id::new(item.id)),
                    )
                    .with_role(item.role.unwrap_or(ribbon.role))
                    .draggable(ribbon.draggable);
                    if let Some(child) = item.child_ribbon {
                        slot_item = slot_item.with_child_ribbon(child);
                    }
                    slot_item.active = is_active(item.id);
                    slot_item
                })
                .collect();
            if slot_items.is_empty() {
                continue;
            }
            resolved.push(ResolvedSlotRibbon {
                id: bevy_mara::vocab::Id::new((ribbon.id, cluster)),
                chrome_id: Some(ribbon.id),
                scope: ribbon_scope(ribbon.id),
                edge: ribbon.edge,
                role: ribbon.role,
                mode: ribbon.mode,
                cluster,
                accepts: ribbon.accepts,
                items: std::mem::take(&mut slot_items),
            });
        }
    }

    host.draw_slot_ribbons_featureful(accent, &resolved, open, placement, drag)
        .into_iter()
        .filter_map(|click| {
            items
                .iter()
                .find(|item| click.item == bevy_mara::vocab::Id::new(item.id))
                .map(|item| RibbonClick { item: item.id })
        })
        .collect()
}

fn ribbon_scope(_ribbon_id: &'static str) -> RibbonScope {
    RibbonScope::View(bevy_mara::ViewId::new("gearbox.viewer"))
}

fn glyph_icon_payload(glyph: RibbonGlyph) -> &'static str {
    let payload = match glyph {
        RibbonGlyph::Text(s) | RibbonGlyph::Icon(s) | RibbonGlyph::Svg(s) => s,
    };
    if bevy_mara::icons::is_icon_payload(payload) {
        payload
    } else {
        "apps"
    }
}

fn item_parts<'a>(
    ribbons: &'a [RibbonDef],
    items: &'a [RibbonItem],
    placement: &RibbonPlacement,
    item: &'static str,
) -> Option<(&'a RibbonDef, RibbonCluster)> {
    let item = items.iter().find(|candidate| candidate.id == item)?;
    let (ribbon_id, cluster, _) =
        placement.resolve_parts(item.id, item.ribbon, item.cluster, item.slot);
    let ribbon = ribbons.iter().find(|candidate| candidate.id == ribbon_id)?;
    Some((ribbon, cluster))
}

fn live_panel_anchor(
    ribbons: &[RibbonDef],
    items: &[RibbonItem],
    placement: &RibbonPlacement,
    item: &'static str,
) -> Option<bevy_mara::pane::PaneAnchor> {
    let (ribbon, cluster) = item_parts(ribbons, items, placement, item)?;
    let zone = match cluster {
        RibbonCluster::Start => bevy_mara::pane::RailZone::Start,
        RibbonCluster::Middle => bevy_mara::pane::RailZone::Middle,
        RibbonCluster::End => bevy_mara::pane::RailZone::End,
    };
    let edge = bevy_mara::phone_remapped_ribbon_edge(ribbon.edge, cluster, ribbon_scope(ribbon.id));
    Some(match edge {
        RibbonEdge::Left => bevy_mara::pane::PaneAnchor::LeftRail(zone),
        RibbonEdge::Right => bevy_mara::pane::PaneAnchor::RightRail(zone),
        RibbonEdge::Top => bevy_mara::pane::PaneAnchor::TopRail(zone),
        RibbonEdge::Bottom => bevy_mara::pane::PaneAnchor::BottomRail(zone),
    })
}

pub fn show_mara_pane_for_item<'spec>(
    ctx: &egui::Context,
    ribbons: &[RibbonDef],
    items: &[RibbonItem],
    placement: &RibbonPlacement,
    item: &'static str,
    title: &'static str,
    accent: impl Into<egui::Color32>,
    add: impl FnOnce(&mut PaneBody<'_, 'spec>),
) {
    let accent: egui::Color32 = accent.into();
    let anchor = live_panel_anchor(ribbons, items, placement, item)
        .unwrap_or(PaneAnchor::LeftRail(RailZone::Start));
    let host = MaraHostCtx::ui_only(ctx, None);
    host.publish_ribbon_pane_ids(
        items
            .iter()
            .filter(|item| item.role.unwrap_or(RibbonRole::Panel) == RibbonRole::Panel)
            .map(|item| MaraId::new(item.id)),
    );
    host.show_pane(
        Pane::new(item, title, anchor, accent).resize(PaneResize::SPAN),
        add,
    );
}

fn panel_pos(
    ctx: &egui::Context,
    ribbons: &[RibbonDef],
    items: &[RibbonItem],
    placement: &RibbonPlacement,
    item: &'static str,
    size: egui::Vec2,
) -> egui::Pos2 {
    let rect = ctx.content_rect();
    let gap = bevy_mara::ribbon_clearance() + 10.0;
    let Some(anchor) = live_panel_anchor(ribbons, items, placement, item) else {
        return rect.left_top() + egui::vec2(gap, 48.0);
    };
    let y_for_zone = |zone| match zone {
        bevy_mara::pane::RailZone::Start => rect.top() + 48.0,
        bevy_mara::pane::RailZone::Middle => rect.center().y - size.y * 0.5,
        bevy_mara::pane::RailZone::End => rect.bottom() - size.y - 48.0,
    };
    let x_for_zone = |zone| match zone {
        bevy_mara::pane::RailZone::Start => rect.left() + 48.0,
        bevy_mara::pane::RailZone::Middle => rect.center().x - size.x * 0.5,
        bevy_mara::pane::RailZone::End => rect.right() - size.x - 48.0,
    };
    match anchor {
        bevy_mara::pane::PaneAnchor::LeftRail(zone) => {
            egui::pos2(rect.left() + gap, y_for_zone(zone))
        }
        bevy_mara::pane::PaneAnchor::RightRail(zone) => {
            egui::pos2(rect.right() - size.x - gap, y_for_zone(zone))
        }
        bevy_mara::pane::PaneAnchor::TopRail(zone) => {
            egui::pos2(x_for_zone(zone), rect.top() + gap)
        }
        bevy_mara::pane::PaneAnchor::BottomRail(zone) => {
            egui::pos2(x_for_zone(zone), rect.bottom() - size.y - gap)
        }
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
    ribbons: &[RibbonDef],
    items: &[RibbonItem],
    placement: &RibbonPlacement,
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
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .fixed_pos(panel_pos(ctx, ribbons, items, placement, item, size))
        .frame(
            egui::Frame::window(&ctx.global_style())
                .fill(egui::Color32::from_rgba_unmultiplied(12, 15, 22, 232))
                .stroke(egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 145),
                ))
                .inner_margin(egui::Margin::same(10)),
        )
        .default_size(size)
        .min_width(size.x.min(220.0))
        .open(keep)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .strong()
                        .color(style::TEXT_PRIMARY),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new("Mara").small().color(accent));
                });
            });
            ui.add_space(style::space::TIGHT);
            ui.separator();
            ui.add_space(style::space::BLOCK);
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
    let accent = accent.into();
    let host = MaraHostCtx::ui_only(ctx, None);
    host.command_palette(state, items, accent)
}
