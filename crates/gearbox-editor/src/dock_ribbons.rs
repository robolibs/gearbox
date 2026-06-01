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
    RibbonCluster, RibbonDef, RibbonDrag, RibbonEdge, RibbonGlyph, RibbonItem, RibbonMode,
    RibbonOpen, RibbonPlacement, RibbonRole, draw_assembly, find_item,
};

use super::style::AccentColor;
use super::usd_load::LoadUsdQueue;

pub const RIBBON_LEFT: &str = "editor_ribbon_left";
pub const RIBBON_RIGHT: &str = "editor_ribbon_right";
pub const RIBBON_TRANSPORT: &str = "editor_ribbon_transport";

pub const ID_WORKSPACE: &str = "workspace";
pub const ID_LIBRARY: &str = "library";
pub const ID_LOAD_USD: &str = "load_usd";
pub const ID_USD_TREE: &str = "usd_tree";
pub const ID_INSPECTOR: &str = "inspector";
pub const ID_PROPERTIES: &str = "properties";

// Viewer-style panel buttons added on the LEFT rail's End cluster
// (bottom of the rail). The host binary owns each panel's draw fn;
// the editor only declares the buttons + exposes the IDs so
// `is_menu_open(...)` works cross-crate.
pub const ID_OVERLAYS: &str = "overlays";
pub const ID_TIMELINE: &str = "timeline";
pub const ID_INFO: &str = "stage_info";
pub const ID_LOG: &str = "log";
pub const ID_KEYS: &str = "keys";
pub const ID_VARIANTS: &str = "variants";
pub const ID_CAMERAS: &str = "cameras";
pub const ID_MATERIALS: &str = "materials";

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
        glyph: RibbonGlyph::Text("W"),
        tooltip: "Workspace",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_LIBRARY,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 1,
        glyph: RibbonGlyph::Text("L"),
        tooltip: "Library",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_LOAD_USD,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 2,
        // Folder glyph reads as "open file" without a panel body.
        glyph: RibbonGlyph::Text("📂"),
        tooltip: "Load USD…  —  pick a .usd / .usda / .usdc / .usdz",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_USD_TREE,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 3,
        glyph: RibbonGlyph::Text("🌳"),
        tooltip: "USD prim tree — full hierarchy of every loaded asset",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_INSPECTOR,
        ribbon: RIBBON_RIGHT,
        cluster: RibbonCluster::Start,
        slot: 0,
        glyph: RibbonGlyph::Text("I"),
        tooltip: "Inspector",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_PROPERTIES,
        ribbon: RIBBON_RIGHT,
        cluster: RibbonCluster::Start,
        slot: 1,
        glyph: RibbonGlyph::Text("P"),
        tooltip: "Properties",
        child_ribbon: None,
    },
    // ─── End-cluster (bottom of LEFT rail) ────────────────────────
    RibbonItem {
        id: ID_OVERLAYS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 0,
        glyph: RibbonGlyph::Text("O"),
        tooltip: "Overlays — toggles for grid / axes / wireframe / physics gizmos / colliders",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_TIMELINE,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 1,
        glyph: RibbonGlyph::Text("⏱"),
        tooltip: "Timeline — stage-time playback (▶ / ⏮ / scrub)",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_INFO,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 2,
        glyph: RibbonGlyph::Text("i"),
        tooltip: "Stage info — counts of prims / lights / physics / variants",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_LOG,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 3,
        glyph: RibbonGlyph::Text("📜"),
        tooltip: "Loader log — recent USD / physics / animation messages",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_KEYS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 4,
        glyph: RibbonGlyph::Text("?"),
        tooltip: "Keyboard shortcuts",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_VARIANTS,
        ribbon: RIBBON_RIGHT,
        cluster: RibbonCluster::End,
        slot: 0,
        glyph: RibbonGlyph::Text("V"),
        tooltip: "Variants — author-time selection sets per prim",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_CAMERAS,
        ribbon: RIBBON_RIGHT,
        cluster: RibbonCluster::End,
        slot: 1,
        glyph: RibbonGlyph::Text("C"),
        tooltip: "Cameras — view bookmarks + USD-authored camera mounts",
        child_ribbon: None,
    },
    RibbonItem {
        id: ID_MATERIALS,
        ribbon: RIBBON_RIGHT,
        cluster: RibbonCluster::End,
        slot: 2,
        glyph: RibbonGlyph::Text("M"),
        tooltip: "Materials — live colour / roughness / metallic per material",
        child_ribbon: None,
    },
];

/// Is the button identified by `id` the currently-open menu on
/// whichever ribbon + cluster it lives on (after any user drag)?
pub fn is_menu_open(open: &RibbonOpen, placement: &RibbonPlacement, id: &'static str) -> bool {
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
    mut load_usd: ResMut<LoadUsdQueue>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let clicks = draw_assembly(
        ctx,
        accent.0,
        RIBBONS,
        RIBBON_ITEMS,
        &mut open,
        &mut placement,
        &mut drag,
        |_| false,
    );
    // Action buttons (no panel) — handle the click and immediately
    // un-toggle the auto-opened state so the button doesn't visually
    // stick on with no panel body to fill the slot.
    for c in clicks {
        if c.item == ID_LOAD_USD {
            if let Some(item) = find_item(RIBBON_ITEMS, ID_LOAD_USD) {
                let (rid, _, _) = placement.resolve(item);
                // `draw_assembly` has already toggled this OPEN; we
                // toggle once more to flip it back to closed.
                open.toggle(rid, ID_LOAD_USD);
            }
            if let Some(files) = rfd::FileDialog::new()
                .add_filter("USD", &["usd", "usda", "usdc", "usdz"])
                .pick_files()
            {
                for f in files {
                    load_usd.0.push(f);
                }
            }
        }
    }
}
