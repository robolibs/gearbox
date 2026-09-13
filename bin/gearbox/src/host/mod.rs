//! The mara window host. mara owns the window, the ribbon rails and the
//! panes; Bevy renders the simulator into the viewport behind them. Panes
//! read the Bevy world directly between frames and ask it for anything
//! query-shaped through `HostCommands`.

pub mod capture;
pub mod keys;
pub mod panes;
pub mod replay;

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Mutex;

use bevy::prelude::*;
use mara::host::{MaraHostCtx, RibbonRail};
use mara::ui::mara_core;
use mara::ui::modules::bevy as mara_bevy;
use mara::window::{CreationContext, WindowApp};
use mara_core::pane::{PaneAnchor, RailZone};
use mara_core::ribbon::RibbonAction;
use mara_core::style::{AccentColor, GlassOpacity, Mode, active_accent};
use mara_core::vocab::Id as MaraId;
use mara_core::{CommandPaletteState, PaletteItem, RibbonAvoidance, WorkspaceStack};

use crate::viewer::commands::{HostCommand, HostCommands};
use crate::viewer::log::LoaderLog;
use crate::viewer::overlays::DisplayToggles;
use crate::viewer::state::ReloadRequest;
use crate::viewer::tf_overlay::TfLabels;
use panes::{Outbox, PaneCtx};

pub const RIBBON_LEFT: &str = "gearbox_left";

pub const PANE_SELECTION: &str = "gearbox_pane_selection";
pub const PANE_OUTLINER: &str = "gearbox_pane_outliner";
pub const PANE_AGENTS: &str = "gearbox_pane_agents";
pub const PANE_INFO: &str = "gearbox_pane_info";
pub const PANE_CAMERAS: &str = "gearbox_pane_cameras";
pub const PANE_CONTROLLERS: &str = "gearbox_pane_controllers";
pub const PANE_OVERLAYS: &str = "gearbox_pane_overlays";
pub const PANE_TIMELINE: &str = "gearbox_pane_timeline";
pub const PANE_KEYS: &str = "gearbox_pane_keys";
pub const PANE_LOG: &str = "gearbox_pane_log";
pub const PANE_MACHINE: &str = "gearbox_pane_machine";

const ACTION_PLAY: &str = "gearbox_action_play";
const ACTION_CLEAR: &str = "gearbox_action_clear";

fn ribbon_action(id: &'static str) -> RibbonAction {
    RibbonAction::Command(MaraId::new(id))
}

/// What `main` hands the window app, since mara constructs it.
struct HostInit {
    cli_paths: Vec<PathBuf>,
    log: LoaderLog,
}

static INIT: Mutex<Option<HostInit>> = Mutex::new(None);

pub fn run(cli_paths: Vec<PathBuf>, log: LoaderLog) -> Result<(), Box<dyn std::error::Error>> {
    *INIT.lock().unwrap() = Some(HostInit { cli_paths, log });
    mara::window::AppRunner::new()
        .title("gearbox — USD simulator")
        .size(1400.0, 900.0)
        .run::<GearboxApp>()
}

/// `GEARBOX_OPEN_PANEL=agents|machine|selection|info|overlays|log|tree`
/// picks the pane open at start, for scripted runs.
fn default_panes() -> &'static str {
    match std::env::var("GEARBOX_OPEN_PANEL").as_deref() {
        Ok("agents") => PANE_AGENTS,
        Ok("machine") => PANE_MACHINE,
        Ok("selection") => PANE_SELECTION,
        Ok("info") => PANE_INFO,
        Ok("overlays") => PANE_OVERLAYS,
        Ok("log") => PANE_LOG,
        _ => PANE_OUTLINER,
    }
}

pub struct GearboxApp {
    view: mara_bevy::MaraBevyViewport,
    workspace: WorkspaceStack,
    log: LoaderLog,
    palette: CommandPaletteState,
    keys: keys::KeyBridge,
    capture: capture::WindowCapture,
    default_left: &'static str,
}

impl WindowApp for GearboxApp {
    fn new(ctx: CreationContext<'_>) -> Self {
        replay::install(ctx.__internal_egui_ctx());
        let init = INIT.lock().unwrap().take();
        let (cli_paths, log) = match init {
            Some(init) => (init.cli_paths, init.log),
            None => (Vec::new(), LoaderLog::default()),
        };
        let mut view = mara_bevy::MaraBevyViewport::with_render_state_plugins_and_content(
            ctx.__internal_render_state(),
            crate::app::configure_plugins,
            move |app: &mut App| crate::app::configure(app, cli_paths.clone()),
        );
        // A simulator animates on its own: never wait for pointer input.
        view.set_continuous_rendering(true);
        let default_left = default_panes();
        Self {
            view,
            workspace: WorkspaceStack::new("gearbox-workspace"),
            log,
            palette: CommandPaletteState::default(),
            keys: keys::KeyBridge::default(),
            capture: capture::WindowCapture::default(),
            default_left,
        }
    }

    fn update(&mut self, host: &mut MaraHostCtx<'_>) {
        mara_core::style::set_theme(mara_core::style::theme_pro(Mode::Dark));
        host.apply_theme(AccentColor::default(), GlassOpacity::default());
        let accent = active_accent();
        let egui = host.__internal_egui().clone();
        let Self {
            view,
            workspace,
            log,
            palette,
            keys,
            capture,
            default_left,
        } = self;
        capture.update(&egui);

        // Input goes into the world before the viewport ticks it.
        if let Some(world) = view.world_mut() {
            keys.forward(&egui, world);
            if !egui.egui_wants_keyboard_input() {
                hotkeys(&egui, host, world, palette);
            }
            panes::machine::auto_open(host, world);
        }

        let viewport = {
            let mut vctx = host.view_ctx(workspace, accent, RibbonAvoidance::all());
            view.show(&mut vctx, host.gpu(), accent);
            vctx.screen_rect()
        };

        let Some(world) = view.world_mut() else {
            return;
        };
        let physics_on = world.resource::<gearbox_api::PhysicsActive>().0;
        let outbox = Outbox::default();
        let world = RefCell::new(world);
        let ctx = PaneCtx {
            accent,
            egui: &egui,
            outbox: outbox.clone(),
            log,
        };

        let left = RibbonRail::view_left(RIBBON_LEFT, "gearbox.ribbons")
            .default_open(default_left)
            .pane(
                PANE_MACHINE,
                "vehicle-tractor",
                "Machine",
                PaneAnchor::LeftRail(RailZone::Start),
                |body| panes::machine::show(body, &mut world.borrow_mut(), &ctx),
            )
            .pane(
                PANE_SELECTION,
                "folder-open",
                "Selection",
                PaneAnchor::LeftRail(RailZone::Start),
                |body| panes::selection::show(body, &mut world.borrow_mut(), &ctx),
            )
            .pane(
                PANE_OUTLINER,
                "cube-tree",
                "Outliner",
                PaneAnchor::LeftRail(RailZone::Start),
                |body| panes::outliner::show(body, &mut world.borrow_mut(), &ctx),
            )
            .pane(
                PANE_AGENTS,
                "vehicle-tractor",
                "Agents",
                PaneAnchor::LeftRail(RailZone::Start),
                |body| panes::agents::show(body, &mut world.borrow_mut(), &ctx),
            )
            .pane(
                PANE_INFO,
                "info",
                "Stage info",
                PaneAnchor::LeftRail(RailZone::Start),
                |body| panes::info::show(body, &mut world.borrow_mut(), &ctx),
            )
            .pane(
                PANE_CAMERAS,
                "camera",
                "Cameras",
                PaneAnchor::LeftRail(RailZone::Start),
                |body| panes::cameras::show(body, &mut world.borrow_mut(), &ctx),
            )
            .pane(
                PANE_CONTROLLERS,
                "options",
                "Machine controllers",
                PaneAnchor::LeftRail(RailZone::Start),
                |body| panes::controllers::show(body, &mut world.borrow_mut(), &ctx),
            )
            .action_in(
                mara_core::ribbon::RibbonCluster::Middle,
                ACTION_PLAY,
                if physics_on { "pause" } else { "play" },
                "Play / pause physics",
                ribbon_action(ACTION_PLAY),
            )
            .action_in(
                mara_core::ribbon::RibbonCluster::Middle,
                ACTION_CLEAR,
                "broom",
                "Clear scene (unload every runtime USD)",
                ribbon_action(ACTION_CLEAR),
            )
            .pane(
                PANE_OVERLAYS,
                "color",
                "Overlays",
                PaneAnchor::LeftRail(RailZone::End),
                |body| panes::overlays::show(body, &mut world.borrow_mut(), &ctx),
            )
            .pane(
                PANE_TIMELINE,
                "clock",
                "Timeline",
                PaneAnchor::LeftRail(RailZone::End),
                |body| panes::timeline::show(body, &mut world.borrow_mut(), &ctx),
            )
            .pane(
                PANE_KEYS,
                "keyboard",
                "Controls",
                PaneAnchor::LeftRail(RailZone::End),
                |body| panes::keys::show(body, &ctx),
            )
            .pane(
                PANE_LOG,
                "document-text",
                "Log",
                PaneAnchor::LeftRail(RailZone::End),
                |body| panes::log::show(body, &ctx),
            );
        let clicks = host.show_ribbon_rail(left, accent);

        for click in clicks {
            tracing::debug!(target: "gearbox", "ribbon click {:?} {:?}", click.item, click.action);
            if click.action == ribbon_action(ACTION_PLAY) {
                outbox.push(HostCommand::SetPhysics(!physics_on));
            } else if click.action == ribbon_action(ACTION_CLEAR) {
                outbox.push(HostCommand::Clear);
            }
        }

        if let Some(id) = host.command_palette(palette, PALETTE_ITEMS, accent) {
            palette_action(id, host, &mut world.borrow_mut(), &outbox);
            palette.open = false;
        }

        paint_tf_labels(&egui, viewport, &world.borrow());
        world
            .borrow_mut()
            .resource_mut::<HostCommands>()
            .0
            .extend(outbox.drain());
    }
}

/// Single-key shortcuts while no text field has the keyboard: pane toggles,
/// overlay toggles, reload, and the command palette on Ctrl+K / Ctrl+P.
fn hotkeys(
    egui: &egui::Context,
    host: &MaraHostCtx<'_>,
    world: &mut World,
    palette: &mut CommandPaletteState,
) {
    use egui::Key;
    let (ctrl, pressed) = egui.input(|i| {
        let hit = |key: Key| {
            i.events.iter().any(|event| {
                matches!(event, egui::Event::Key { key: k, pressed: true, repeat: false, .. } if *k == key)
            })
        };
        let ctrl = i.events.iter().any(|event| {
            matches!(event, egui::Event::Key { pressed: true, modifiers, .. } if modifiers.ctrl)
        }) || i.modifiers.ctrl;
        let keys = [
            Key::T,
            Key::N,
            Key::I,
            Key::O,
            Key::M,
            Key::F,
            Key::Slash,
            Key::Questionmark,
            Key::G,
            Key::X,
            Key::P,
            Key::B,
            Key::Y,
            Key::C,
            Key::R,
            Key::K,
        ];
        (ctrl, keys.map(hit))
    });
    let [t, n, i, o, m, f, slash, question, g, x, p, b, y, c, r, k] = pressed;
    if ctrl {
        if k || p {
            palette.open = !palette.open;
            if palette.open {
                palette.query.clear();
                palette.selected = 0;
            }
        }
        return;
    }
    let toggles: [(bool, &'static str, &'static str); 7] = [
        (t, RIBBON_LEFT, PANE_OUTLINER),
        (n, RIBBON_LEFT, PANE_AGENTS),
        (i, RIBBON_LEFT, PANE_INFO),
        (o, RIBBON_LEFT, PANE_OVERLAYS),
        (f, RIBBON_LEFT, PANE_SELECTION),
        (slash || question, RIBBON_LEFT, PANE_KEYS),
        (m, RIBBON_LEFT, PANE_MACHINE),
    ];
    for (hit, rail, pane) in toggles {
        if hit {
            host.toggle_rail_pane(rail, pane);
        }
    }
    let mut display = world.resource_mut::<DisplayToggles>();
    if g {
        display.show_world_grid = !display.show_world_grid;
    }
    if x {
        display.show_world_axes = !display.show_world_axes;
    }
    if p {
        display.show_prim_markers = !display.show_prim_markers;
    }
    if b {
        display.show_skeleton = !display.show_skeleton;
    }
    if y {
        display.show_physics = !display.show_physics;
    }
    if c {
        display.show_colliders = !display.show_colliders;
    }
    if r {
        world.resource_mut::<ReloadRequest>().requested = true;
    }
}

const PALETTE_ITEMS: &[PaletteItem] = &[
    PaletteItem {
        id: "open_selection",
        label: "Open: Selection",
        hint: Some("F"),
    },
    PaletteItem {
        id: "open_tree",
        label: "Open: Outliner",
        hint: Some("T"),
    },
    PaletteItem {
        id: "open_agents",
        label: "Open: Agents",
        hint: Some("N"),
    },
    PaletteItem {
        id: "open_info",
        label: "Open: Stage info",
        hint: Some("I"),
    },
    PaletteItem {
        id: "open_cameras",
        label: "Open: Cameras",
        hint: None,
    },
    PaletteItem {
        id: "open_controllers",
        label: "Open: Machine controllers",
        hint: None,
    },
    PaletteItem {
        id: "open_machine",
        label: "Open: Machine",
        hint: Some("M"),
    },
    PaletteItem {
        id: "open_overlays",
        label: "Open: Overlays",
        hint: Some("O"),
    },
    PaletteItem {
        id: "open_timeline",
        label: "Open: Timeline",
        hint: None,
    },
    PaletteItem {
        id: "open_keys",
        label: "Open: Controls",
        hint: Some("?"),
    },
    PaletteItem {
        id: "open_log",
        label: "Open: Log",
        hint: None,
    },
    PaletteItem {
        id: "toggle_grid",
        label: "Toggle: Ground grid",
        hint: Some("G"),
    },
    PaletteItem {
        id: "toggle_axes",
        label: "Toggle: World axes",
        hint: Some("X"),
    },
    PaletteItem {
        id: "toggle_markers",
        label: "Toggle: Prim markers",
        hint: Some("P"),
    },
    PaletteItem {
        id: "toggle_wireframe",
        label: "Toggle: Wireframe",
        hint: None,
    },
    PaletteItem {
        id: "toggle_tf",
        label: "Toggle: TF tree (frames, names, links)",
        hint: None,
    },
    PaletteItem {
        id: "toggle_physics",
        label: "Physics: Play / pause",
        hint: None,
    },
    PaletteItem {
        id: "reload_stage",
        label: "Stage: Reload",
        hint: Some("R"),
    },
    PaletteItem {
        id: "browse_usd",
        label: "Stage: Add USD…",
        hint: None,
    },
    PaletteItem {
        id: "clear_scene",
        label: "Scene: Clear",
        hint: None,
    },
];

fn palette_action(id: &str, host: &MaraHostCtx<'_>, world: &mut World, outbox: &Outbox) {
    let open = |rail, pane| host.set_rail_pane_open(rail, pane, true);
    match id {
        "open_selection" => open(RIBBON_LEFT, PANE_SELECTION),
        "open_tree" => open(RIBBON_LEFT, PANE_OUTLINER),
        "open_agents" => open(RIBBON_LEFT, PANE_AGENTS),
        "open_info" => open(RIBBON_LEFT, PANE_INFO),
        "open_cameras" => open(RIBBON_LEFT, PANE_CAMERAS),
        "open_controllers" => open(RIBBON_LEFT, PANE_CONTROLLERS),
        "open_machine" => open(RIBBON_LEFT, PANE_MACHINE),
        "open_overlays" => open(RIBBON_LEFT, PANE_OVERLAYS),
        "open_timeline" => open(RIBBON_LEFT, PANE_TIMELINE),
        "open_keys" => open(RIBBON_LEFT, PANE_KEYS),
        "open_log" => open(RIBBON_LEFT, PANE_LOG),
        "toggle_grid" => {
            let mut t = world.resource_mut::<DisplayToggles>();
            t.show_world_grid = !t.show_world_grid;
        }
        "toggle_axes" => {
            let mut t = world.resource_mut::<DisplayToggles>();
            t.show_world_axes = !t.show_world_axes;
        }
        "toggle_markers" => {
            let mut t = world.resource_mut::<DisplayToggles>();
            t.show_prim_markers = !t.show_prim_markers;
        }
        "toggle_wireframe" => {
            let mut t = world.resource_mut::<DisplayToggles>();
            t.wireframe = !t.wireframe;
        }
        "toggle_tf" => {
            let mut t = world.resource_mut::<DisplayToggles>();
            let on = !(t.show_tf_frames || t.show_tf_links);
            t.show_tf_frames = on;
            t.show_tf_names = on;
            t.show_tf_links = on;
        }
        "toggle_physics" => {
            let on = world.resource::<gearbox_api::PhysicsActive>().0;
            outbox.push(HostCommand::SetPhysics(!on));
        }
        "reload_stage" => outbox.push(HostCommand::Reload),
        "browse_usd" => {
            if let Some(path) = panes::pick_usd_file() {
                outbox.push(HostCommand::Load(path));
            }
        }
        "clear_scene" => outbox.push(HostCommand::Clear),
        _ => {}
    }
}

/// TF link names over the viewport, where the Bevy side projected them.
fn paint_tf_labels(egui: &egui::Context, viewport: mara_core::vocab::Rect, world: &World) {
    let Some(labels) = world.get_resource::<TfLabels>() else {
        return;
    };
    if labels.0.is_empty() {
        return;
    }
    let rect: egui::Rect = viewport.into();
    let painter = egui
        .layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("gearbox_tf_names"),
        ))
        .with_clip_rect(rect);
    let font = egui::FontId::proportional(12.0);
    let fg = egui::Color32::from_rgb(255, 235, 140);
    let bg = egui::Color32::from_black_alpha(150);
    for label in &labels.0 {
        let pos = rect.min
            + egui::vec2(
                label.at[0] * rect.width() + 6.0,
                label.at[1] * rect.height() - 6.0,
            );
        let galley = painter.layout_no_wrap(label.name.clone(), font.clone(), fg);
        let bounds = egui::Rect::from_min_size(pos, galley.size()).expand(2.0);
        painter.rect_filled(bounds, 2.0, bg);
        painter.galley(pos, galley, fg);
    }
}
