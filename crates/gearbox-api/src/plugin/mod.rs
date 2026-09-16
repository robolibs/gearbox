//! Bevy plugins: the host bus as a resource, polled every frame.

pub mod loader;
pub mod marker;

use std::collections::HashMap;

use bevy::prelude::*;

use crate::host::{HostBus, HostConfig};
use crate::machine::MachineAgent;
use crate::wire::*;

pub use loader::{MachineDeleteQueue, MachineLoadQueue, PendingLoadedUsd, UsdLoaderPlugin};
pub use marker::UsdMarkerPlugin;

/// Scene-wide clear. Written by the API and by the UI, read by everything
/// that owns spawned entities.
#[derive(Message, Default, Debug, Clone, Copy)]
pub struct SimResetRequest {
    pub pause_clock: bool,
    pub scope: u32,
}

#[derive(Resource, Debug, Clone)]
pub struct UsdAssetRoot(pub std::path::PathBuf);

/// Whether the physics world steps this frame: the clock the play button
/// and `/gearbox/scene/clock` flip. Starts paused.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct PhysicsActive(pub bool);

/// Objects the host reports on `/gearbox/scene/list`. Each owner refreshes
/// its own list: the binary for loaded assets, the loader plugin for props,
/// the marker plugin for markers.
#[derive(Resource, Default, Debug, Clone)]
pub struct SceneObjects {
    pub machines: Vec<SceneObject>,
    pub props: Vec<SceneObject>,
    pub markers: Vec<SceneObject>,
}

impl SceneObjects {
    pub fn all(&self) -> impl Iterator<Item = &SceneObject> {
        self.machines
            .iter()
            .chain(self.props.iter())
            .chain(self.markers.iter())
    }

    pub fn len(&self) -> usize {
        self.machines.len() + self.props.len() + self.markers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Selection mirrored between the API and the viewer. `from_api` flags a
/// change the viewer has not yet applied.
#[derive(Resource, Default, Debug, Clone)]
pub struct SelectionState {
    pub kind: u32,
    pub id: String,
    pub from_api: bool,
}

#[derive(Resource)]
pub struct GearboxBus {
    pub host: HostBus,
    pub machines: HashMap<String, MachineAgent>,
    pub physics_steps: u64,
    /// Frames left to run before a `step` request pauses the clock again.
    step_budget: u32,
    shutdown: bool,
    last_clock: Option<ClockState>,
}

impl GearboxBus {
    pub fn instance(&self) -> &str {
        &self.host.config.instance
    }

    pub fn machine_refs(&self) -> Vec<MachineRef> {
        let mut refs: Vec<_> = self.machines.values().map(|m| m.machine_ref()).collect();
        refs.sort_by_key(|r| r.machine_id());
        refs
    }

    pub fn publish_event(&mut self, event: SceneEvent) {
        self.host.publish_event(&event);
    }
}

pub struct GearboxBusPlugin {
    pub config: HostConfig,
}

impl Plugin for GearboxBusPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SimResetRequest>()
            .init_resource::<SceneObjects>()
            .init_resource::<SelectionState>();
        match HostBus::new(self.config.clone()) {
            Ok(host) => {
                info!("{}", host.banner());
                app.insert_resource(GearboxBus {
                    host,
                    machines: HashMap::new(),
                    physics_steps: 0,
                    step_budget: 0,
                    shutdown: false,
                    last_clock: None,
                });
                app.add_systems(
                    Update,
                    (
                        serve_host_core,
                        poll_machine_agents,
                        publish_clock,
                        shutdown_if_requested,
                    )
                        .chain(),
                );
            }
            Err(err) => {
                error!("gearbox-api: host agent failed to start: {err}; tool API disabled");
            }
        }
    }
}

fn serve_host_core(
    mut bus: ResMut<GearboxBus>,
    mut physics: ResMut<PhysicsActive>,
    objects: Res<SceneObjects>,
    mut selection: ResMut<SelectionState>,
    mut reset: MessageWriter<SimResetRequest>,
    mut exit: MessageWriter<AppExit>,
) {
    if physics.0 {
        bus.physics_steps += 1;
    }
    let bus = bus.as_mut();
    if bus.step_budget > 0 {
        bus.step_budget -= 1;
        if bus.step_budget == 0 {
            physics.0 = false;
        }
    }
    let uptime = bus.host.uptime_ms();
    let paused = !physics.0;
    let steps = bus.physics_steps;
    let machine_count = bus.machines.len() as u32;
    let object_count = objects.len() as u32;
    let allowed = bus
        .host
        .agent
        .allowed_peers()
        .iter()
        .filter_map(|id| agentio::did_key::endpoint_to_did_key(id).ok())
        .collect::<Vec<_>>()
        .join(",");
    let allow_any = bus.host.agent.allows_any_peer().to_string();
    let props = Props::from_pairs(&[
        ("name", &bus.host.config.instance),
        ("version", &bus.host.config.version),
        ("did", &bus.host.did()),
        ("allowed", &allowed),
        ("allow_any", &allow_any),
        ("log", bus.host.config.log.as_deref().unwrap_or("")),
    ]);
    bus.host.serve_info(|_| HostInfo {
        uptime_ms: uptime,
        pid: std::process::id(),
        paused: paused as u32,
        machine_count,
        object_count,
        props: props.to_bytes(),
    });

    let mut active = physics.0;
    let mut shutdown = bus.shutdown;
    let mut step_budget = bus.step_budget;
    bus.host.serve_clock(|cmd| {
        match cmd.op {
            clock_op::PAUSE => {
                active = false;
                step_budget = 0;
            }
            clock_op::PLAY => {
                active = true;
                step_budget = 0;
            }
            clock_op::TOGGLE => active = !active,
            clock_op::SHUTDOWN => shutdown = true,
            clock_op::STEP => {
                active = true;
                step_budget = cmd.steps.max(1);
            }
            _ => {}
        }
        ClockState {
            step: steps,
            paused: (!active) as u32,
            _pad: 0,
        }
    });
    bus.host.serve_clear(|req| {
        reset.write(SimResetRequest {
            pause_clock: req.pause_clock != 0,
            scope: req.scope,
        });
        if req.pause_clock != 0 {
            active = false;
        }
        Status::ok()
    });
    if shutdown && !bus.shutdown {
        exit.write(AppExit::Success);
    }
    bus.shutdown = shutdown;
    bus.step_budget = step_budget;
    if active != physics.0 {
        physics.0 = active;
    }

    bus.host.serve_list(|q| {
        objects
            .all()
            .filter(|o| q.kind == object_kind::ANY || o.kind == q.kind)
            .map(|o| SceneObject {
                x: o.x,
                y: o.y,
                z: o.z,
                yaw_deg: o.yaw_deg,
                kind: o.kind,
                props: o.props.clone(),
            })
            .collect()
    });

    let sel = selection.as_mut();
    bus.host.serve_select(|req| {
        if req.query == 0 {
            sel.kind = req.kind;
            sel.id = req.id();
            sel.from_api = true;
        }
        if sel.id.is_empty() {
            Selection::default()
        } else {
            Selection::set(sel.kind, &sel.id)
        }
    });

    let refs = bus.machine_refs();
    bus.host.serve_machines(|_| {
        refs.iter()
            .map(|r| MachineRef {
                props: r.props.clone(),
            })
            .collect()
    });
}

fn poll_machine_agents(mut bus: ResMut<GearboxBus>) {
    for agent in bus.machines.values_mut() {
        agent.poll();
    }
}

fn publish_clock(mut bus: ResMut<GearboxBus>, physics: Res<PhysicsActive>) {
    let state = ClockState {
        step: bus.physics_steps,
        paused: (!physics.0) as u32,
        _pad: 0,
    };
    let changed = bus
        .last_clock
        .as_ref()
        .map(|last| last.paused != state.paused)
        .unwrap_or(true);
    if changed {
        bus.host.publish_clock(&state);
        bus.last_clock = Some(state);
    }
}

fn shutdown_if_requested(bus: Res<GearboxBus>, mut exit: MessageWriter<AppExit>) {
    if bus.shutdown {
        exit.write(AppExit::Success);
    }
}
