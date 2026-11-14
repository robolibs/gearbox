//! One owner per DOF, all-or-nothing command batches, explicit authority
//! transfer, expiry, and the per-tick resolution into one actuator state.
//!
//! ```text
//! submit -> validate whole batch -> accept -> (per tick) expire -> hold capture
//!        -> clamp to limits -> cap -> ActuatorState -> one commit write
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use super::policy::CapPolicy;
use super::trace::{Trace, TraceEvent};
use super::types::*;
use crate::machine_model::{DriveType, Interface};

/// The authored drive of a DOF, SI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveSpec {
    pub stiffness: f64,
    pub damping: f64,
    pub drive: DriveType,
    pub max_force: Option<f64>,
}

/// What activation knows about a DOF.
#[derive(Clone, Debug, PartialEq)]
pub struct DofSpec {
    pub key: DofKey,
    pub interfaces: Vec<Interface>,
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    pub drive: Option<DriveSpec>,
    pub hardware_cap: Option<f64>,
    /// The owner that holds the DOF from activation.
    pub default_owner: Option<OwnerId>,
}

#[derive(Clone, Debug, PartialEq)]
struct Lease {
    owner: OwnerId,
    epoch: u64,
}

#[derive(Clone, Debug, PartialEq)]
enum Expiry {
    Never,
    Tick(u64),
    Wall(Instant),
}

#[derive(Clone, Debug, PartialEq)]
struct Accepted {
    setpoint: Setpoint,
    expiry: Expiry,
    requested_cap: Option<f64>,
}

#[derive(Clone, Debug)]
struct DofState {
    spec: DofSpec,
    lease: Option<Lease>,
    /// Owners waiting under a direct override, most recent last.
    suspended: Vec<Lease>,
    command: Option<Accepted>,
    /// The coordinate a hold took when it began.
    captured: Option<f64>,
    last_sequence: BTreeMap<OwnerId, u64>,
}

/// A server-issued authority over a set of DOFs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub owner: OwnerId,
    pub epoch: u64,
    pub dofs: Vec<DofKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub owner: OwnerId,
    pub epoch: u64,
    pub sequence: u64,
    pub commands: Vec<(DofKey, Setpoint)>,
    pub validity: Validity,
    pub requested_cap: Option<f64>,
}

/// What an owner's DOF does when its command lapses or its owner leaves.
/// An internal servo keeps its own gains: a velocity servo stops, a
/// position servo stays on its target.
fn safe_setpoint(previous: Option<&Setpoint>) -> Setpoint {
    match previous {
        Some(Setpoint::Velocity { .. }) => Setpoint::Velocity { qdot: 0.0 },
        Some(Setpoint::Effort { .. }) | None => Setpoint::Free,
        Some(Setpoint::Servo { q: None, stiffness, damping, drive, .. }) => {
            Setpoint::Servo { q: None, qdot: 0.0, stiffness: *stiffness, damping: *damping, drive: *drive }
        }
        Some(servo @ Setpoint::Servo { .. }) => *servo,
        Some(_) => Setpoint::Hold,
    }
}

#[derive(Debug, Default)]
pub struct Arbiter {
    dofs: BTreeMap<DofKey, DofState>,
    next_epoch: u64,
    pub safety_cap: Option<f64>,
    pub trace: Trace,
    tick: u64,
}

impl Arbiter {
    pub fn new() -> Self {
        Self { next_epoch: 1, ..Default::default() }
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn spec(&self, key: &DofKey) -> Option<&DofSpec> {
        self.dofs.get(key).map(|s| &s.spec)
    }

    pub fn owner(&self, key: &DofKey) -> Option<&OwnerId> {
        self.dofs.get(key).and_then(|s| s.lease.as_ref()).map(|l| &l.owner)
    }

    pub fn dofs(&self) -> impl Iterator<Item = &DofKey> {
        self.dofs.keys()
    }

    fn epoch(&mut self) -> u64 {
        let e = self.next_epoch.max(1);
        self.next_epoch = e + 1;
        e
    }

    /// Activate a machine generation's DOFs and their default owners, all or
    /// none. A DOF of the machine already registered is replaced.
    pub fn register(&mut self, specs: Vec<DofSpec>) -> Result<(), Vec<Rejection>> {
        let mut errors = Vec::new();
        let mut seen = BTreeSet::new();
        for spec in &specs {
            if !seen.insert(spec.key.clone()) {
                errors.push(Rejection::DuplicateDof { dof: spec.key.to_string() });
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let machines: BTreeSet<&str> = specs.iter().map(|s| s.key.machine.as_str()).collect();
        let machines: Vec<String> = machines.into_iter().map(str::to_string).collect();
        for machine in &machines {
            self.unregister(machine);
        }
        for spec in specs {
            let lease = spec.default_owner.clone().map(|owner| Lease { owner, epoch: 0 });
            self.dofs.insert(
                spec.key.clone(),
                DofState { spec, lease, suspended: Vec::new(), command: None, captured: None, last_sequence: BTreeMap::new() },
            );
        }
        Ok(())
    }

    /// Add or replace one DOF without touching the rest of its machine.
    pub fn insert(&mut self, spec: DofSpec) {
        let lease = spec.default_owner.clone().map(|owner| Lease { owner, epoch: 0 });
        self.dofs.insert(
            spec.key.clone(),
            DofState { spec, lease, suspended: Vec::new(), command: None, captured: None, last_sequence: BTreeMap::new() },
        );
    }

    pub fn remove(&mut self, key: &DofKey) {
        self.dofs.remove(key);
    }

    /// Drop a machine's DOFs, leases and commands: reset, reload, detach.
    pub fn unregister(&mut self, machine: &str) {
        let before = self.dofs.len();
        self.dofs.retain(|k, _| k.machine != machine);
        if before != self.dofs.len() {
            self.trace.push(self.tick, || TraceEvent::Invalidated { machine: machine.to_string() });
        }
    }

    /// Clear every command, hold and lease override of one machine and give
    /// each DOF back to its default owner, keeping the generation.
    pub fn reset_machine(&mut self, machine: &str) {
        for state in self.dofs.values_mut().filter(|s| s.spec.key.machine == machine) {
            state.command = None;
            state.captured = None;
            state.suspended.clear();
            state.last_sequence.clear();
            state.lease = state.spec.default_owner.clone().map(|owner| Lease { owner, epoch: 0 });
        }
        self.trace.push(self.tick, || TraceEvent::Invalidated { machine: machine.to_string() });
    }

    /// A direct override of `dofs`: every DOF must exist, allow the
    /// interfaces asked for, and be free of another direct lease. The
    /// previous owners are suspended and the DOFs hold where they are.
    pub fn acquire(&mut self, owner_name: &str, machine: &str, dofs: &[DofKey]) -> Result<Grant, Rejection> {
        for key in dofs {
            let state = self.dofs.get(key).ok_or_else(|| Rejection::UnknownDof(key.to_string()))?;
            if state.spec.interfaces.is_empty() {
                return Err(Rejection::InterfaceNotAllowed { dof: key.to_string(), interface: "any".into() });
            }
            if let Some(lease) = &state.lease
                && lease.owner.class == OwnerClass::Direct
            {
                return Err(Rejection::Conflict { dof: key.to_string(), owners: vec![lease.owner.to_string()] });
            }
        }
        let epoch = self.epoch();
        let owner = OwnerId::new(OwnerClass::Direct, machine, &format!("{owner_name}#{epoch}"));
        for key in dofs {
            let state = self.dofs.get_mut(key).expect("checked");
            if let Some(previous) = state.lease.take() {
                state.suspended.push(previous);
            }
            state.lease = Some(Lease { owner: owner.clone(), epoch });
            state.command = Some(Accepted { setpoint: Setpoint::Hold, expiry: Expiry::Never, requested_cap: None });
            state.captured = None;
        }
        self.trace.push(self.tick, || TraceEvent::Acquired { owner: owner.to_string(), epoch, dofs: dofs.iter().map(ToString::to_string).collect() });
        Ok(Grant { owner, epoch, dofs: dofs.to_vec() })
    }

    /// End a direct lease: its DOFs hold where they are, and the suspended
    /// owner resumes only when `resume` says so.
    pub fn release(&mut self, grant: &Grant, resume: bool) {
        for key in &grant.dofs {
            let Some(state) = self.dofs.get_mut(key) else { continue };
            if state.lease.as_ref().is_none_or(|l| l.owner != grant.owner || l.epoch != grant.epoch) {
                continue;
            }
            let previous = state.command.as_ref().map(|c| c.setpoint);
            state.lease = if resume { state.suspended.pop() } else { None };
            let setpoint = if resume { Setpoint::Hold } else { safe_setpoint(previous.as_ref()) };
            state.command = Some(Accepted { setpoint, expiry: Expiry::Never, requested_cap: None });
            state.captured = None;
            state.last_sequence.remove(&grant.owner);
        }
        self.trace.push(self.tick, || TraceEvent::Released { owner: grant.owner.to_string(), epoch: grant.epoch, resumed: resume });
    }

    /// Validate a batch whole, then accept every command or none.
    pub fn submit(&mut self, batch: Batch, now: Instant) -> Result<(), Rejection> {
        let result = self.validate(&batch);
        match &result {
            Ok(()) => {
                let expiry = match batch.validity {
                    Validity::Latched => Expiry::Never,
                    Validity::Ticks(n) => Expiry::Tick(self.tick + n),
                    Validity::WallMs(ms) => Expiry::Wall(now + Duration::from_millis(ms)),
                };
                for (key, setpoint) in &batch.commands {
                    let state = self.dofs.get_mut(key).expect("validated");
                    let entering_hold = matches!(setpoint, Setpoint::Hold) && !matches!(state.command.as_ref().map(|c| c.setpoint), Some(Setpoint::Hold));
                    if entering_hold || !matches!(setpoint, Setpoint::Hold) {
                        state.captured = None;
                    }
                    state.command = Some(Accepted { setpoint: *setpoint, expiry: expiry.clone(), requested_cap: batch.requested_cap });
                    match state.last_sequence.get_mut(&batch.owner) {
                        Some(last) => *last = batch.sequence,
                        None => {
                            state.last_sequence.insert(batch.owner.clone(), batch.sequence);
                        }
                    }
                }
                self.trace.push(self.tick, || TraceEvent::Accepted { owner: batch.owner.to_string(), sequence: batch.sequence, dofs: batch.commands.len() });
            }
            Err(reason) => {
                self.trace.push(self.tick, || TraceEvent::Rejected { owner: batch.owner.to_string(), sequence: batch.sequence, reason: reason.to_string() });
            }
        }
        result
    }

    fn validate(&self, batch: &Batch) -> Result<(), Rejection> {
        let mut seen = BTreeSet::new();
        let external = batch.owner.class == OwnerClass::Direct;
        for (key, setpoint) in &batch.commands {
            if batch.commands.len() > 1 && !seen.insert(key) {
                return Err(Rejection::DuplicateDof { dof: key.to_string() });
            }
            let Some(state) = self.dofs.get(key) else {
                if let Some(other) = self.dofs.keys().find(|k| k.machine == key.machine && k.joint == key.joint && k.dof == key.dof) {
                    return Err(Rejection::StaleGeneration { dof: key.to_string(), have: other.generation, given: key.generation });
                }
                return Err(Rejection::UnknownDof(key.to_string()));
            };
            match &state.lease {
                Some(lease) if lease.owner == batch.owner => {
                    if external && lease.epoch != batch.epoch {
                        return Err(Rejection::StaleEpoch { given: batch.epoch, current: lease.epoch });
                    }
                }
                Some(lease) => {
                    return Err(Rejection::NotOwner { dof: key.to_string(), owner: batch.owner.to_string(), holder: lease.owner.to_string() });
                }
                None if batch.owner.class == OwnerClass::TestDriver => {}
                None if external => return Err(Rejection::Revoked),
                None => {
                    return Err(Rejection::NotOwner { dof: key.to_string(), owner: batch.owner.to_string(), holder: "nobody".into() });
                }
            }
            if let Some(&last) = state.last_sequence.get(&batch.owner)
                && batch.sequence <= last
            {
                return Err(Rejection::OutOfOrder { given: batch.sequence, last });
            }
            if !setpoint.is_finite() {
                return Err(Rejection::NonFinite { dof: key.to_string() });
            }
            if external {
                if matches!(setpoint, Setpoint::Servo { .. }) {
                    return Err(Rejection::ServoFromExternal { dof: key.to_string() });
                }
                if let Some(interface) = setpoint.interface()
                    && !state.spec.interfaces.contains(&interface)
                {
                    return Err(Rejection::InterfaceNotAllowed { dof: key.to_string(), interface: interface.as_str().into() });
                }
                if matches!(setpoint, Setpoint::Velocity { .. } | Setpoint::Effort { .. }) && batch.validity == Validity::Latched {
                    return Err(Rejection::UnboundedLiveCommand { dof: key.to_string() });
                }
            }
        }
        Ok(())
    }

    /// Commands that lapsed at `now` fall to their safe setpoint.
    fn expire(&mut self, now: Instant) {
        let tick = self.tick;
        let mut lapsed = Vec::new();
        for (key, state) in &mut self.dofs {
            let Some(command) = &state.command else { continue };
            let gone = match command.expiry {
                Expiry::Never => false,
                Expiry::Tick(t) => tick >= t,
                Expiry::Wall(at) => now >= at,
            };
            if gone {
                let safe = safe_setpoint(Some(&command.setpoint));
                if matches!(safe, Setpoint::Hold) {
                    state.captured = None;
                }
                state.command = Some(Accepted { setpoint: safe, expiry: Expiry::Never, requested_cap: None });
                lapsed.push(key.to_string());
            }
        }
        for dof in lapsed {
            self.trace.push(tick, || TraceEvent::Expired { dof });
        }
    }

    /// Revoke an owner's leases at once: lapsed sessions, detach. Its DOFs
    /// enter their safe setpoint before the next physics step.
    pub fn revoke(&mut self, owner: &OwnerId) {
        for state in self.dofs.values_mut() {
            if state.lease.as_ref().is_some_and(|l| &l.owner == owner) {
                let previous = state.command.as_ref().map(|c| c.setpoint);
                state.lease = None;
                state.command = Some(Accepted { setpoint: safe_setpoint(previous.as_ref()), expiry: Expiry::Never, requested_cap: None });
                state.captured = None;
            }
            state.suspended.retain(|l| &l.owner != owner);
        }
        self.trace.push(self.tick, || TraceEvent::Revoked { owner: owner.to_string() });
    }

    /// The DOFs `owner` holds drop to their safe setpoint and keep their
    /// owner: a slave's master-driven joints once it is detached.
    pub fn quiesce(&mut self, owner: &OwnerId) {
        for state in self.dofs.values_mut() {
            if state.lease.as_ref().is_some_and(|l| &l.owner == owner)
                && let Some(command) = &state.command
                && !matches!(command.setpoint, Setpoint::Hold | Setpoint::Free)
            {
                let safe = safe_setpoint(Some(&command.setpoint));
                state.command = Some(Accepted { setpoint: safe, expiry: Expiry::Never, requested_cap: None });
                state.captured = None;
            }
        }
        self.trace.push(self.tick, || TraceEvent::Revoked { owner: format!("{owner} (quiesced)") });
    }

    /// One physics tick: lapse expired commands, capture holds from the
    /// measured coordinates, clamp to limits and caps. Every DOF with a
    /// command yields exactly one actuator state.
    pub fn resolve(&mut self, now: Instant, measured: impl Fn(&DofKey) -> Option<f64>) -> Vec<(DofKey, ActuatorState)> {
        self.tick += 1;
        self.expire(now);
        let safety = self.safety_cap;
        let mut out = Vec::new();
        for (key, state) in &mut self.dofs {
            let Some(command) = &state.command else { continue };
            let clamp = |q: f64| {
                let q = state.spec.lower.map_or(q, |l| q.max(l));
                state.spec.upper.map_or(q, |u| q.min(u))
            };
            let caps = CapPolicy {
                authored: state.spec.drive.and_then(|d| d.max_force),
                hardware: state.spec.hardware_cap,
                requested: command.requested_cap,
                safety,
            };
            let cap = caps.effective();
            let drive = state.spec.drive;
            // Without an authored drive the targets still reach a motor device
            // that realizes them; a bare joint gets no gain.
            let servo = |q: Option<f64>, qdot: f64| match drive {
                Some(d) => Effective::Servo { q, qdot, stiffness: if q.is_some() { d.stiffness } else { 0.0 }, damping: d.damping, drive: d.drive },
                None => Effective::Servo { q, qdot, stiffness: 0.0, damping: 0.0, drive: DriveType::Force },
            };
            let effective = match command.setpoint {
                Setpoint::Position { q, qdot } => servo(Some(clamp(q)), qdot.unwrap_or(0.0)),
                Setpoint::Velocity { qdot } => servo(None, qdot),
                Setpoint::Effort { force } => Effective::Effort { force: cap.map_or(force, |c| force.clamp(-c, c)) },
                Setpoint::Free => Effective::Free,
                Setpoint::Hold => {
                    if state.captured.is_none() {
                        state.captured = measured(key);
                    }
                    match state.captured {
                        Some(q) => servo(Some(clamp(q)), 0.0),
                        None => Effective::Free,
                    }
                }
                Setpoint::Servo { q, qdot, stiffness, damping, drive } => Effective::Servo { q: q.map(clamp), qdot, stiffness, damping, drive },
            };
            out.push((key.clone(), ActuatorState { effective, cap }));
        }
        let tick = self.tick;
        for (key, state) in &out {
            self.trace.push(tick, || TraceEvent::Effective { dof: key.to_string(), state: *state });
        }
        out
    }

    /// The setpoint a DOF's owner asked for, before resolution.
    pub fn requested(&self, key: &DofKey) -> Option<Setpoint> {
        self.dofs.get(key).and_then(|s| s.command.as_ref()).map(|c| c.setpoint)
    }
}
