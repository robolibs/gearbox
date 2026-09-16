//! The mara window host. mara owns the window, the ribbon rails and the
//! panes; Bevy renders the simulator into the viewport behind them. Panes
//! read the Bevy world directly between frames and ask it for anything
//! query-shaped through `HostCommands`.

pub mod capture;
pub mod gizmo;
pub mod keys;
mod machine_context;
mod ground_card;
pub mod panes;
pub mod replay;

use std::path::PathBuf;
use std::sync::Mutex;

use bevy::prelude::*;
use mara::host::{MaraHostCtx, RibbonRail};
use mara::ui::mara_core;
use mara::ui::modules::bevy as mara_bevy;
use mara::window::{CreationContext, WindowApp};
use mara_core::ribbon::RibbonAction;
use mara_core::shelf::{ShelfContainer, ShelfDef, ShelfEdge};
use mara_core::style::{AccentColor, GlassOpacity, Mode, active_accent};
use mara_core::vocab::Id as MaraId;
use mara_core::{CommandPaletteState, PaletteItem, RibbonAvoidance, ShellBar, ShellEvent, WorkspaceStack};

use crate::viewer::commands::{HostCommand, HostCommands};
use crate::viewer::log::LoaderLog;
use crate::viewer::overlays::DisplayToggles;
use crate::viewer::state::ReloadRequest;
use crate::viewer::tf_overlay::TfLabels;
use panes::{Outbox, PaneCtx};

pub const RIBBON_LEFT: &str = "gearbox_left";

/// Three docked Shelves — real reserved-space chrome, not ribbon-opened
/// floating panes. Left holds Scene/View/Environment/Capture/Controls;
/// Right holds the Machine pane; Bottom holds Shortcuts.
pub const SHELF_LEFT: &str = "gearbox_shelf_left";
pub const SHELF_RIGHT: &str = "gearbox_shelf_right";
pub const SHELF_BOTTOM: &str = "gearbox_shelf_bottom";

/// These no longer name a rail-opened pane; they just namespace each
/// pane module's own `cid`/`pid` calls (unchanged since before the shelf).
pub const PANE_MACHINE: &str = "gearbox_pane_machine";
pub const PANE_SCENE: &str = "gearbox_pane_scene";
pub const PANE_VIEW: &str = "gearbox_pane_view";
pub const PANE_LOG: &str = "gearbox_pane_log";
pub const PANE_ENVIRONMENT: &str = "gearbox_pane_environment";
pub const PANE_CAPTURE: &str = "gearbox_pane_capture";
pub const PANE_SHORTCUTS: &str = "gearbox_pane_shortcuts";

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
        .title("gearbox — robotics simulator")
        .size(window_size().0, window_size().1)
        .run::<GearboxApp>()
}

/// `GEARBOX_WINDOW=WxH` sizes the window in points; 1400x900 by default.
fn window_size() -> (f32, f32) {
    std::env::var("GEARBOX_WINDOW")
        .ok()
        .and_then(|value| {
            let (w, h) = value.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((1400.0, 900.0))
}

pub struct GearboxApp {
    view: mara_bevy::MaraBevyViewport,
    workspace: WorkspaceStack,
    log: LoaderLog,
    palette: CommandPaletteState,
    keys: keys::KeyBridge,
    capture: capture::WindowCapture,
    gizmo: gizmo::PoseGizmo,
    machine_context: machine_context::MachineContext,
    shelf_state: mara_core::ShelfState,
}

impl WindowApp for GearboxApp {
    fn new(ctx: CreationContext<'_>) -> Self {
        replay::install(ctx.__internal_egui_ctx());
        let init = INIT.lock().unwrap().take();
        let (cli_paths, log) = match init {
            Some(init) => (init.cli_paths, init.log),
            None => (Vec::new(), LoaderLog::default()),
        };
        let wireframe_supported = ctx.__internal_render_state().is_some_and(|state| {
            use bevy::render::settings::WgpuFeatures;
            state.device.limits().max_immediate_size >= 16
                && state.device.features().contains(WgpuFeatures::POLYGON_MODE_LINE | WgpuFeatures::IMMEDIATES)
        });
        let mut view = mara_bevy::MaraBevyViewport::with_render_state_plugins_and_content(
            ctx.__internal_render_state(),
            crate::app::configure_plugins,
            move |app: &mut App| crate::app::configure(app, cli_paths.clone(), wireframe_supported),
        );
        // A simulator animates on its own: never wait for pointer input.
        view.set_continuous_rendering(true);
        Self {
            view,
            workspace: WorkspaceStack::new("gearbox-workspace"),
            log,
            palette: CommandPaletteState::default(),
            keys: keys::KeyBridge::default(),
            capture: capture::WindowCapture::default(),
            gizmo: gizmo::PoseGizmo::default(),
            machine_context: machine_context::MachineContext::default(),
            shelf_state: {
                let mut state = mara_core::ShelfState::default();
                state.set_edge_visible(ShelfEdge::Left, false);
                state.set_edge_visible(ShelfEdge::Right, false);
                state.set_edge_visible(ShelfEdge::Bottom, false);
                state
            },
        }
    }

    fn configure_shell(&mut self, bar: &mut ShellBar) {
        bar.app_menu = false;
    }

    fn on_shell_event(&mut self, event: ShellEvent, _ctx: &mut MaraHostCtx<'_>) {
        match event {
            ShellEvent::LeftShelfToggled => self.shelf_state.toggle_edge_visible(ShelfEdge::Left),
            ShellEvent::RightShelfToggled => self.shelf_state.toggle_edge_visible(ShelfEdge::Right),
            ShellEvent::BottomShelfToggled => {
                self.shelf_state.toggle_edge_visible(ShelfEdge::Bottom)
            }
            _ => {}
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
            gizmo,
            machine_context,
            shelf_state,
        } = self;
        capture.update(&egui);

        // Input goes into the world before the viewport ticks it.
        if let Some(world) = view.world_mut() {
            world.resource_mut::<crate::viewer::drive::HostInputFocus>().0 = egui.input(|i| i.focused);
            keys.forward(&egui, world);
            if !egui.egui_wants_keyboard_input() {
                hotkeys(&egui, world, palette, shelf_state);
            }
            machine_context.interact(&egui, world);
            if !world.resource::<crate::viewer::machine_context::MachineHover>().captures_pointer {
                gizmo.interact(&egui, world);
            } else {
                world.resource_mut::<crate::viewer::systems::GizmoGrab>().0 = false;
            }
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
        let ctx = PaneCtx {
            accent,
            egui: &egui,
            outbox: outbox.clone(),
            log,
        };

        // Three docked Shelves: Left is one tabbed container swapping
        // between Scene/View/Environment/Capture/Controls, Right is the
        // Machine pane, Bottom is Shortcuts.
        let left = ShelfContainer::tabbed(
            MaraId::new(SHELF_LEFT),
            "Left",
            "list",
            vec![
                panes::scene::tab(world, &ctx),
                panes::view::tab(world, &ctx),
                panes::environment::tab(world, &ctx),
                panes::capture::tab(world, &ctx),
                panes::controls::tab(world, &ctx),
            ],
        );
        let shelves = vec![
            ShelfDef::new(SHELF_LEFT, ShelfEdge::Left, accent)
                .default_size(340.0)
                .movable()
                .container(left),
            ShelfDef::new(SHELF_RIGHT, ShelfEdge::Right, accent)
                .default_size(340.0)
                .movable()
                .container(panes::machine::container(world, &ctx)),
            ShelfDef::new(SHELF_BOTTOM, ShelfEdge::Bottom, accent)
                .default_size(220.0)
                .container(panes::shortcuts::container(world, &ctx)),
        ];
        let layout = host.layout_shelves(&shelves, shelf_state);
        let responses = host.show_shelves(layout, shelves, shelf_state);

        panes::machine::apply(&responses, world, &ctx);
        panes::scene::apply(&responses, world, &ctx);
        panes::view::apply(&responses, world, &ctx);
        panes::environment::apply(&responses, world, &ctx);
        panes::capture::apply(&responses, world, &ctx);
        panes::controls::apply(&responses, world, &ctx);
        panes::shortcuts::apply(&responses, world, &ctx);

        let left = RibbonRail::view_left(RIBBON_LEFT, "gearbox.ribbons")
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
                "Clear scene",
                ribbon_action(ACTION_CLEAR),
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
            palette_action(id, world, &outbox, shelf_state);
            palette.open = false;
        }

        paint_tf_labels(&egui, viewport, world);
        gizmo.paint(&egui, viewport);
        machine_context.show(&egui, viewport.into(), world);
        world.resource_mut::<HostCommands>().0.extend(outbox.drain());
    }
}

/// Single-key shortcuts while no text field has the keyboard: the left
/// sidebar, overlay toggles, reload, and the command palette on
/// Ctrl+K / Ctrl+P.
fn hotkeys(
    egui: &egui::Context,
    world: &mut World,
    palette: &mut CommandPaletteState,
    shelf_state: &mut mara_core::ShelfState,
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
            Key::O,
            Key::M,
            Key::F,
            Key::Slash,
            Key::Questionmark,
            Key::G,
            Key::X,
            Key::P,
            Key::Y,
            Key::C,
            Key::R,
            Key::K,
        ];
        (ctrl, keys.map(hit))
    });
    let [o, m, f, slash, question, g, x, p, y, c, r, k] = pressed;
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
    if f || o {
        shelf_state.toggle_edge_visible(ShelfEdge::Left);
    }
    if m {
        shelf_state.toggle_edge_visible(ShelfEdge::Right);
    }
    if slash || question {
        shelf_state.toggle_edge_visible(ShelfEdge::Bottom);
    }
    let mut display = world.resource_mut::<DisplayToggles>();
    if g {
        display.show_world_grid = !display.show_world_grid;
    }
    if x {
        display.show_world_axes = !display.show_world_axes;
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
    PaletteItem { id: "open_machine", label: "Open: Machine", hint: Some("M") },
    PaletteItem { id: "open_scene", label: "Open: Scene", hint: Some("F") },
    PaletteItem { id: "open_view", label: "Open: View", hint: Some("O") },
    PaletteItem { id: "open_environment", label: "Open: Environment", hint: None },
    PaletteItem { id: "open_controller", label: "Open: Controller", hint: None },
    PaletteItem { id: "open_shortcuts", label: "Open: Shortcuts", hint: Some("?") },
    PaletteItem { id: "toggle_grid", label: "Toggle: Ground grid", hint: Some("G") },
    PaletteItem { id: "toggle_axes", label: "Toggle: World axes", hint: Some("X") },
    PaletteItem { id: "toggle_wireframe", label: "Toggle: Wireframe", hint: None },
    PaletteItem { id: "toggle_tf", label: "Toggle: TF tree (frames, names, links)", hint: None },
    PaletteItem { id: "toggle_physics", label: "Physics: Play / pause", hint: None },
    PaletteItem { id: "reload_stage", label: "Scene: Reload", hint: Some("R") },
    PaletteItem { id: "browse_usd", label: "Scene: Load USD…", hint: None },
    PaletteItem { id: "clear_scene", label: "Scene: Clear", hint: None },
];

fn palette_action(id: &str, world: &mut World, outbox: &Outbox, shelf_state: &mut mara_core::ShelfState) {
    match id {
        "open_machine" => {
            shelf_state.set_edge_visible(ShelfEdge::Right, true);
        }
        "open_scene" | "open_view" | "open_environment" | "open_controller" => {
            shelf_state.set_edge_visible(ShelfEdge::Left, true);
        }
        "open_shortcuts" => {
            shelf_state.set_edge_visible(ShelfEdge::Bottom, true);
        }
        "toggle_grid" => {
            let mut t = world.resource_mut::<DisplayToggles>();
            t.show_world_grid = !t.show_world_grid;
        }
        "toggle_axes" => {
            let mut t = world.resource_mut::<DisplayToggles>();
            t.show_world_axes = !t.show_world_axes;
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
