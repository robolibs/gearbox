//! Shared ribbon assembly for the editor's Left and Right docks.
//!
//! Declares both rails in one place so the app-level button layout
//! lives independently of each dock's panel-content system. Both
//! ribbons are `TwoSided` (Start cluster at the top corner, End
//! cluster at the bottom corner), `Panel` role (buttons open
//! exclusive menus), and drag-accept each other so the user can
//! move any button across rails or between clusters.
//!
//! Initial layout: every button starts in its rail's `Start`
//! cluster. The `End` cluster is declared but empty — dropping a
//! button onto the bottom corner of either rail parks it there.

use bevy::prelude::*;
use bevy_egui::EguiContexts;
use bevy_frost::{
    draw_assembly, find_item, RibbonCluster, RibbonDef, RibbonDrag, RibbonEdge, RibbonItem,
    RibbonMode, RibbonOpen, RibbonPlacement, RibbonRole,
};

use super::style::AccentColor;

pub const RIBBON_LEFT: &str = "editor_ribbon_left";
pub const RIBBON_RIGHT: &str = "editor_ribbon_right";
pub const RIBBON_TRANSPORT: &str = "editor_ribbon_transport";

pub const ID_WORKSPACE: &str = "workspace";
pub const ID_LIBRARY: &str = "library";
pub const ID_INSPECTOR: &str = "inspector";
pub const ID_PROPERTIES: &str = "properties";

/// All ribbons the editor declares — two vertical Panel rails
/// (draggable, cross-accepting) plus a horizontal Icon transport
/// bar at the top (centred, locked in place, rejects drops).
///
/// Every `draw_assembly` call in the editor passes this entire
/// slice so `compute_side_insets` can see all three edges and
/// the top bar inset stops at the side rails.
pub const RIBBONS: &[RibbonDef] = &[
    RibbonDef {
        id: RIBBON_LEFT,
        edge: RibbonEdge::Left,
        role: RibbonRole::Panel,
        mode: RibbonMode::TwoSided,
        draggable: true,
        accepts: &[RIBBON_RIGHT],
    },
    RibbonDef {
        id: RIBBON_RIGHT,
        edge: RibbonEdge::Right,
        role: RibbonRole::Panel,
        mode: RibbonMode::TwoSided,
        draggable: true,
        accepts: &[RIBBON_LEFT],
    },
    RibbonDef {
        id: RIBBON_TRANSPORT,
        edge: RibbonEdge::Top,
        role: RibbonRole::Icon,
        mode: RibbonMode::Centered,
        draggable: false,
        accepts: &[],
    },
];

/// Initial button layout — all four buttons live in their rail's
/// `Start` (upper) cluster. End clusters start empty.
pub const RIBBON_ITEMS: &[RibbonItem] = &[
    RibbonItem {
        id: ID_WORKSPACE,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 0,
        glyph: "W",
        tooltip: "Workspace",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_LIBRARY,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 1,
        glyph: "L",
        tooltip: "Library",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_INSPECTOR,
        ribbon: RIBBON_RIGHT,
        cluster: RibbonCluster::Start,
        slot: 0,
        glyph: "I",
        tooltip: "Inspector",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_PROPERTIES,
        ribbon: RIBBON_RIGHT,
        cluster: RibbonCluster::Start,
        slot: 1,
        glyph: "P",
        tooltip: "Properties",
        child_ribbon: None,
    },
];

/// Is the button identified by `id` the currently-open menu on
/// whichever ribbon + cluster it lives on (after any user drag)?
pub fn is_menu_open(
    open: &RibbonOpen,
    placement: &RibbonPlacement,
    id: &'static str,
) -> bool {
    let Some(item) = find_item(RIBBON_ITEMS, id) else {
        return false;
    };
    let (rid, _, _) = placement.resolve(item);
    open.is_open(rid, id)
}

/// Draw the dock ribbons + dispatch button toggles. Runs every
/// frame in the egui pass. Panel rendering happens in
/// `left_dock_ui` / `right_dock_ui`.
pub fn draw_dock_ribbons_ui(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut open: ResMut<RibbonOpen>,
    mut placement: ResMut<RibbonPlacement>,
    mut drag: ResMut<RibbonDrag>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let _ = draw_assembly(
        ctx,
        accent.0,
        RIBBONS,
        RIBBON_ITEMS,
        &mut open,
        &mut placement,
        &mut drag,
        |_| false,
    );
}
