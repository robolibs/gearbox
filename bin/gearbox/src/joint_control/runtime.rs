//! Activation of accepted machines in the joint command contract: their DOFs
//! bind to the built physics joints and their default owners register once
//! every joint exists; rejected or removed machines are unbound.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::controller::ControllerInventory;
use crate::physics::PhysicsWorld;
use crate::prim_index::PrimIndex;

/// Frames a machine waited for its joints, to report a stuck activation once.
#[derive(Resource, Default)]
pub struct BindingWaits(pub HashMap<String, u32>);

/// Order of the work inside one `ControlTick`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlSet {
    Clock,
    Controllers,
    Services,
}

/// Which controllers run on the current physics tick, and the simulated
/// time since each last ran: `updateRateHz` by a phase accumulator.
#[derive(Resource, Default, Debug, Clone)]
pub struct ControlClock {
    pub tick: u64,
    pub tick_dt: f64,
    cadences: HashMap<(String, String), super::Cadence>,
    last: HashMap<(String, String), u64>,
    due: HashMap<(String, String), f64>,
}

impl ControlClock {
    /// The simulated seconds since the controller last ran, when it runs on this tick.
    pub fn due(&self, machine: &str, instance: &str) -> Option<f64> {
        self.due.get(&(machine.to_string(), instance.to_string())).copied()
    }

    /// Advance one tick of `dt` for controllers at their rates on a
    /// `physics_hz` clock; a controller is due on its first tick.
    pub fn advance(&mut self, dt: f64, physics_hz: f64, controllers: impl IntoIterator<Item = ((String, String), f64)>) {
        self.tick += 1;
        self.tick_dt = dt;
        self.due.clear();
        for (key, rate) in controllers {
            let cadence = self
                .cadences
                .entry(key.clone())
                .or_insert_with(|| super::Cadence::new(rate.min(physics_hz), physics_hz).unwrap_or_else(|_| super::Cadence::new(physics_hz, physics_hz).expect("physics rate")));
            if cadence.tick() {
                let previous = self.last.insert(key.clone(), self.tick).unwrap_or(self.tick - 1);
                self.due.insert(key, (self.tick - previous) as f64 * dt);
            }
        }
    }

    /// Forget a machine's cadences: its controllers start over.
    pub fn forget(&mut self, machine: &str) {
        self.cadences.retain(|(m, _), _| m != machine);
        self.last.retain(|(m, _), _| m != machine);
    }
}

pub fn advance_control_clock(mut clock: ResMut<ControlClock>, inventory: Res<ControllerInventory>, physics: Res<PhysicsWorld>) {
    let (dt, hz) = (physics.dt(), physics.step_hz);
    let controllers = inventory.machines.iter().flat_map(move |machine| {
        let rates = machine.model.as_ref();
        machine.controllers.iter().filter(|c| c.enabled).map(move |c| {
            let rate = rates
                .and_then(|m| m.controllers.iter().find(|b| b.instance == c.instance))
                .map_or(hz, |b| b.update_rate_hz.value);
            ((machine.id.clone(), c.instance.clone()), rate)
        })
    });
    clock.advance(dt, hz, controllers);
}

pub fn bind_accepted_machines(
    inventory: Res<ControllerInventory>,
    mut physics: ResMut<PhysicsWorld>,
    mut waits: ResMut<BindingWaits>,
    index: Res<PrimIndex>,
    mut clock: Option<ResMut<ControlClock>>,
) {
    let mut live: HashSet<&str> = HashSet::new();
    for machine in &inventory.machines {
        let (Some(model), Some(root)) = (machine.model.as_ref(), machine.scene_root) else { continue };
        if !model.accepted() || !machine.links.is_valid() {
            continue;
        }
        live.insert(machine.id.as_str());
        if physics.control.machines.get(&machine.id) == Some(&model.generation) {
            continue;
        }
        let built: HashMap<String, crate::physics::backend::JointId> = index
            .joints()
            .filter(|(entity, _, _)| index.is_descendant_of(*entity, root))
            .filter_map(|(entity, path, _)| physics.entity_to_joint.get(&entity).map(|id| (path.to_string(), *id)))
            .collect();
        match physics.bind_machine(&machine.id, model, |path| built.get(path).copied()) {
            Ok(count) => {
                waits.0.remove(&machine.id);
                if let (Some(control), Some(body)) = (model.control_frame.as_ref(), model.control_body)
                    && let Some((_, offset)) = model.frame_in_body(control.frame)
                    && let Some(entity) = index.find(root, &model.bodies[body].path)
                    && let Some(&handle) = physics.entity_to_body.get(&entity)
                {
                    let q = offset.rotation;
                    let turn = crate::physics::chassis::chassis_turn(glam::DQuat::from_xyzw(q.x, q.y, q.z, q.w));
                    if turn.angle_between(glam::DQuat::IDENTITY) > 1e-9 {
                        physics.chassis_turns.insert(handle, turn);
                    } else {
                        physics.chassis_turns.remove(&handle);
                    }
                }
                info!("gearbox-control: `{}` generation {} bound {count} DOF(s)", machine.id, model.generation);
            }
            Err(missing) => {
                let waited = waits.0.entry(machine.id.clone()).or_default();
                *waited += 1;
                if *waited == 240 {
                    warn!("gearbox-control: `{}` still waits for joints {}", machine.id, missing.join(", "));
                }
            }
        }
    }
    let stale: Vec<String> = physics.control.machines.keys().filter(|id| !live.contains(id.as_str())).cloned().collect();
    for id in stale {
        physics.unbind_machine(&id);
        if let Some(clock) = clock.as_mut() {
            clock.forget(&id);
        }
        info!("gearbox-control: `{id}` unbound");
    }
}
