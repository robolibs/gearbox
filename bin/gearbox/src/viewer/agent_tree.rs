//! The Agents pane on the left ribbon: one row per controllable machine the
//! loaded USDs declared, nothing static. Rows are Mara hybrid select rows,
//! as in the old editor: click selects the machine (and opens the Machine
//! pane), double-click flies the camera behind it and gives the viewer its
//! drive, the right-edge radio pins the camera to follow it. The trailing
//! text says who holds the drive; a second container lists the selected
//! machine's controllers.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use bevy_mara::ChaseCamera;
use usd_bevy::UsdPrimRef;

use crate::controller::{ControllerInventory, ControllerKey, ControllerStates, UiDrive};
use crate::viewer::machine_panel::{DRIVE_TYPES, MachinePanel};
use crate::viewer::mara_ui::*;
use crate::viewer::state::{ActiveStage, ChaseCameraFly, FlyTarget, FollowTarget};
use crate::viewer::ui::{
    RIB_AGENTS, RIBBON_ITEMS, RIBBON_LEFT, RIBBONS, Selection, is_panel_open, machine_body_entity,
    pod_response,
};

pub struct AgentTreePlugin;

impl Plugin for AgentTreePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RequestPoll(Timer::from_seconds(0.25, TimerMode::Repeating)))
            .add_systems(Startup, open_from_env)
            .add_systems(Update, serve_camera_requests)
            .add_systems(EguiPrimaryContextPass, draw_agent_tree);
    }
}

#[derive(Resource)]
struct RequestPoll(Timer);

/// `gearbox instance camera …` drops `fly ID`, `follow ID` or `unfollow` in
/// `<registry dir>/<name>.camera`; this does what the pane's rows do.
fn serve_camera_requests(
    time: Res<Time>,
    mut poll: ResMut<RequestPoll>,
    inventory: Res<ControllerInventory>,
    mut follow: ResMut<FollowTarget>,
    mut fly: ResMut<ChaseCameraFly>,
    lookup: AgentLookup,
) {
    if !poll.0.tick(time.delta()).just_finished() {
        return;
    }
    let name = std::env::var("GEARBOX_NAME").unwrap_or_else(|_| "gearbox".to_string());
    let request = gearbox_api::registry::registry_dir().join(format!("{name}.camera"));
    let Ok(text) = std::fs::read_to_string(&request) else {
        return;
    };
    let _ = std::fs::remove_file(&request);
    let mut words = text.split_whitespace();
    let action = words.next().unwrap_or("");
    let root = words.next().and_then(|id| {
        inventory
            .machines
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.scene_root)
    });
    info!("gearbox-viewer: camera request `{}`", text.trim());
    match (action, root) {
        ("fly", Some(root)) => {
            if let Ok(cam) = lookup.cameras.single() {
                let body = machine_body_entity(root, &inventory, &lookup.prims, &lookup.parents);
                fly.target = Some(FlyTarget::new(root, body, cam));
            }
        }
        ("follow", Some(root)) => follow.set(Some(root)),
        ("unfollow", _) => follow.set(None),
        _ => warn!(
            "gearbox-viewer: camera request `{}` not understood",
            text.trim()
        ),
    }
}

/// `GEARBOX_OPEN_PANEL=agents` opens the pane at start, for scripted runs.
fn open_from_env(mut open: ResMut<RibbonOpen>) {
    if std::env::var("GEARBOX_OPEN_PANEL").as_deref() == Ok("agents") {
        open.per_ribbon.insert(RIBBON_LEFT, RIB_AGENTS);
    }
}

#[derive(SystemParam)]
struct AgentLookup<'w, 's> {
    prims: Query<'w, 's, (Entity, &'static UsdPrimRef)>,
    parents: Query<'w, 's, &'static ChildOf>,
    cameras: Query<'w, 's, &'static ChaseCamera>,
}

fn cid(section: &str) -> MaraId {
    MaraId::new(("gearbox", RIB_AGENTS, section.to_string()))
}

fn pid(section: &str, idx: usize) -> MaraId {
    MaraId::new(("gearbox", RIB_AGENTS, section.to_string(), idx))
}

/// Mara keeps the list's selected and pinned rows in egui memory; write the
/// live selection and follow target there so the rows show the truth even
/// when either changed elsewhere (scene click, CLI, despawn).
fn sync_list_memory(
    ctx: &egui::Context,
    pod: MaraId,
    selected: Option<usize>,
    pinned: Option<usize>,
) {
    let sel_key: egui::Id = pod.with(("mara_pod_hybrid_select_list_sel", 0usize)).into();
    let pin_key: egui::Id = pod.with(("mara_pod_hybrid_select_list_pin", 0usize)).into();
    ctx.data_mut(|d| {
        d.insert_persisted(sel_key, selected);
        d.insert_persisted(pin_key, pinned);
    });
}

fn drive_keys(
    root: Entity,
    machine: &crate::controller::MachineInstanceSpec,
) -> Vec<ControllerKey> {
    machine
        .controllers
        .iter()
        .filter(|c| c.enabled && DRIVE_TYPES.contains(&c.controller_type.as_str()))
        .map(|c| ControllerKey::new(root, &machine.id, &c.instance))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn draw_agent_tree(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    inventory: Res<ControllerInventory>,
    states: Res<ControllerStates>,
    mut selection: ResMut<Selection>,
    mut active: ResMut<ActiveStage>,
    mut follow: ResMut<FollowTarget>,
    mut fly: ResMut<ChaseCameraFly>,
    mut panel: ResMut<MachinePanel>,
    mut ui_drive: ResMut<UiDrive>,
    lookup: AgentLookup,
) {
    if !is_panel_open(&open, RIB_AGENTS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col: egui::Color32 = accent.0.into();

    let mut machines: Vec<_> = inventory
        .machines
        .iter()
        .filter_map(|m| m.scene_root.map(|root| (root, m)))
        .collect();
    machines.sort_by(|a, b| a.1.id.cmp(&b.1.id));
    let index_of = |entity: Option<Entity>| {
        entity.and_then(|e| machines.iter().position(|(root, _)| *root == e))
    };
    let list_pod = pid("agents", 0);
    sync_list_memory(
        ctx,
        list_pod,
        index_of(selection.0),
        index_of(follow.entity),
    );

    show_mara_pane_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_AGENTS,
        "Agents",
        accent_col,
        |body| {
            let agents_id = cid("agents");
            if machines.is_empty() {
                body.add_normal(
                    agents_id,
                    "Agents",
                    "vehicle_tractor",
                    vec![
                        Pod::new(pid("agents", 0))
                            .with_readout("machines", "none")
                            .with_readout("hint", "spawn a machine with a controller"),
                    ],
                );
                body.render();
                return;
            }
            let labels: Vec<String> = machines.iter().map(|(_, m)| m.id.clone()).collect();
            let trailing: Vec<String> = machines
                .iter()
                .map(|(root, m)| {
                    let drives = drive_keys(*root, m);
                    let holder = if drives.iter().any(|k| panel.gamepad_on(k)) {
                        " · gamepad"
                    } else if drives.iter().any(|k| panel.holds(k)) {
                        " · viewer"
                    } else {
                        ""
                    };
                    format!("{}{holder}", m.kind.as_deref().unwrap_or("machine"))
                })
                .collect();
            body.add_normal(
                agents_id,
                "Agents",
                "vehicle_tractor",
                vec![Pod::new(pid("agents", 0)).with_hybrid_select_list(
                    labels,
                    Some(trailing),
                    accent_col,
                )],
            );

            let picked = index_of(selection.0).map(|i| machines[i]);
            let controllers_id = cid("controllers");
            let mut pod = Pod::new(pid("controllers", 0));
            match picked {
                Some((root, m)) => {
                    for c in m.controllers.iter().filter(|c| c.enabled) {
                        let key = ControllerKey::new(root, &m.id, &c.instance);
                        let short = c.controller_type.trim_start_matches("builtin:");
                        let value = match states.states.get(&key) {
                            Some(s) if DRIVE_TYPES.contains(&c.controller_type.as_str()) => {
                                format!("{short} · {:.1} m/s", s.linear_speed_mps)
                            }
                            _ => short.to_string(),
                        };
                        pod = pod.with_readout(c.instance.clone(), value);
                    }
                    if m.controllers.iter().all(|c| !c.enabled) {
                        pod = pod.with_readout("controllers", "none enabled");
                    }
                }
                None => pod = pod.with_readout("selected", "nothing"),
            }
            body.add_normal(controllers_id, "Controllers", "options", vec![pod]);

            let responses = body.render();
            let Some(list) =
                pod_response(&responses, agents_id, 0).and_then(|r| r.hybrid_select_lists.first())
            else {
                return;
            };
            if let Some(i) = list.body_clicked
                && let Some((root, _)) = machines.get(i)
            {
                selection.0 = Some(*root);
                active.0 = Some(*root);
            }
            if let Some(i) = list.body_double_clicked
                && let Some((root, m)) = machines.get(i)
            {
                let root = *root;
                selection.0 = Some(root);
                active.0 = Some(root);
                if let Ok(cam) = lookup.cameras.single() {
                    let body =
                        machine_body_entity(root, &inventory, &lookup.prims, &lookup.parents);
                    fly.target = Some(FlyTarget::new(root, body, cam));
                }
                let drives = drive_keys(root, m);
                if let Some(key) = drives.first()
                    && !drives.iter().any(|k| panel.holds(k))
                {
                    panel.set_viewer_drive(&mut ui_drive, key, true);
                }
            }
            if let Some(i) = list.radio_clicked
                && let Some((root, _)) = machines.get(i)
            {
                follow.toggle(*root);
            }
        },
    );
}
