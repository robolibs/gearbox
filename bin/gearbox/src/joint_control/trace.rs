//! Bounded record of every accepted, rejected, expired and effective
//! command, by physics tick, for conformance comparison.

use std::collections::VecDeque;

use serde_json::{Value, json};

use super::types::{ActuatorState, Effective};

#[derive(Clone, Debug, PartialEq)]
pub enum TraceEvent {
    Accepted { owner: String, sequence: u64, dofs: usize },
    Rejected { owner: String, sequence: u64, reason: String },
    Expired { dof: String },
    Acquired { owner: String, epoch: u64, dofs: Vec<String> },
    Released { owner: String, epoch: u64, resumed: bool },
    Revoked { owner: String },
    Invalidated { machine: String },
    Effective { dof: String, state: ActuatorState },
    Committed { dof: String, writes: u32 },
}

#[derive(Debug)]
pub struct Trace {
    events: VecDeque<(u64, TraceEvent)>,
    capacity: usize,
    pub enabled: bool,
}

impl Default for Trace {
    fn default() -> Self {
        Self { events: VecDeque::new(), capacity: 65536, enabled: false }
    }
}

fn effective(state: &ActuatorState) -> Value {
    let cap = state.cap.map_or(Value::Null, |c| json!(c));
    match state.effective {
        Effective::Free => json!({ "mode": "free", "cap": cap }),
        Effective::Effort { force } => json!({ "mode": "effort", "force": force, "cap": cap }),
        Effective::Servo { q, qdot, stiffness, damping, drive } => json!({
            "mode": "servo", "q": q, "qdot": qdot, "stiffness": stiffness, "damping": damping,
            "drive": drive.as_str(), "cap": cap,
        }),
    }
}

impl Trace {
    /// Records the event `event` builds, built only while enabled.
    pub fn push(&mut self, tick: u64, event: impl FnOnce() -> TraceEvent) {
        if !self.enabled {
            return;
        }
        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        self.events.push_back((tick, event()));
    }

    pub fn events(&self) -> impl Iterator<Item = &(u64, TraceEvent)> {
        self.events.iter()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// The effective state of every DOF on every tick, in order.
    pub fn effective_targets(&self) -> Vec<(u64, String, ActuatorState)> {
        self.events
            .iter()
            .filter_map(|(tick, e)| match e {
                TraceEvent::Effective { dof, state } => Some((*tick, dof.clone(), *state)),
                _ => None,
            })
            .collect()
    }

    pub fn to_json_lines(&self) -> String {
        let mut out = String::new();
        for (tick, event) in &self.events {
            let value = match event {
                TraceEvent::Accepted { owner, sequence, dofs } => json!({ "tick": tick, "event": "accepted", "owner": owner, "sequence": sequence, "dofs": dofs }),
                TraceEvent::Rejected { owner, sequence, reason } => json!({ "tick": tick, "event": "rejected", "owner": owner, "sequence": sequence, "reason": reason }),
                TraceEvent::Expired { dof } => json!({ "tick": tick, "event": "expired", "dof": dof }),
                TraceEvent::Acquired { owner, epoch, dofs } => json!({ "tick": tick, "event": "acquired", "owner": owner, "epoch": epoch, "dofs": dofs }),
                TraceEvent::Released { owner, epoch, resumed } => json!({ "tick": tick, "event": "released", "owner": owner, "epoch": epoch, "resumed": resumed }),
                TraceEvent::Revoked { owner } => json!({ "tick": tick, "event": "revoked", "owner": owner }),
                TraceEvent::Invalidated { machine } => json!({ "tick": tick, "event": "invalidated", "machine": machine }),
                TraceEvent::Effective { dof, state } => json!({ "tick": tick, "event": "effective", "dof": dof, "state": effective(state) }),
                TraceEvent::Committed { dof, writes } => json!({ "tick": tick, "event": "committed", "dof": dof, "writes": writes }),
            };
            out.push_str(&value.to_string());
            out.push('\n');
        }
        out
    }
}
