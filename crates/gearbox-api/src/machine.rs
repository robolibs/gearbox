//! A machine agent: one agentio `Agent` per simulated machine, hosting the
//! control and telemetry topics for that machine under its own identity.

use std::time::{Duration, Instant};

use agentio::{Agent, DirectoryMode, IdentitySource, Registered};
use datapod::robot::Twist;
use peerbus::{EndpointId, Publisher, ReqServer};

use crate::host::serve_req;
use crate::topics::{self, machine_topic};
use crate::wire::*;

#[derive(Debug, Clone, Default)]
pub struct ControllerDesc {
    pub instance: String,
    pub controller_type: String,
    pub command_interface: Option<String>,
    pub state_interfaces: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MachineConfig {
    pub instance: String,
    pub namespace: String,
    pub machine_id: String,
    pub kind: String,
    pub controllers: Vec<ControllerDesc>,
    pub ephemeral: bool,
    pub allow: Vec<String>,
    pub allow_any: bool,
    pub relay: bool,
}

impl MachineConfig {
    pub fn new(instance: &str, namespace: &str) -> Self {
        Self {
            instance: instance.to_string(),
            namespace: namespace.to_string(),
            machine_id: namespace.to_string(),
            kind: String::new(),
            controllers: Vec::new(),
            ephemeral: false,
            allow: Vec::new(),
            allow_any: false,
            relay: false,
        }
    }

    pub fn with_cmd_vel(mut self, instance: &str, controller_type: &str) -> Self {
        self.controllers.push(ControllerDesc {
            instance: instance.to_string(),
            controller_type: controller_type.to_string(),
            command_interface: Some("cmd_vel".to_string()),
            state_interfaces: vec!["pose".to_string(), "velocity".to_string()],
        });
        self
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: u64,
    pub holder: String,
    pub since: Instant,
    pub last_cmd: Instant,
    pub hold: Duration,
}

/// Silence longer than this after the last command releases the session.
/// A holder that has been silent this long is presumed gone. The hold time
/// zeroes the twist much sooner; this only frees the machine for others.
const AUTO_RELEASE: Duration = Duration::from_secs(60);
const DEFAULT_HOLD: Duration = Duration::from_millis(500);

pub struct MachineAgent {
    pub agent: Agent,
    pub config: MachineConfig,
    info: Registered<ReqServer<Env, Env>>,
    claim: Registered<ReqServer<Env, Env>>,
    release: Registered<ReqServer<Env, Env>>,
    session_q: Registered<ReqServer<Env, Env>>,
    cmd_vel: Registered<ReqServer<Env, Env>>,
    cmd: Registered<ReqServer<Env, Env>>,
    state_pub: Registered<Publisher<Env>>,
    odom_pub: Registered<Publisher<Env>>,
    session: Option<Session>,
    next_session: u64,
    twist: Twist,
    /// Controller commands received since the last drain.
    pub commands: Vec<ControllerCommand>,
}

impl MachineAgent {
    pub fn new(config: MachineConfig, host: EndpointId) -> agentio::Result<Self> {
        let identity = if config.ephemeral {
            IdentitySource::Random
        } else {
            IdentitySource::Name(format!("gearbox/{}/{}", config.instance, config.namespace))
        };
        let mut builder = Agent::builder()
            .name(format!("{}/{}", config.instance, config.namespace))
            .identity(identity)
            .bootstrap([host])
            .directory(DirectoryMode::FrontDoor(host))
            .local_config(crate::host::shm_limits())
            .allow_peers(config.allow.iter().cloned());
        if config.allow_any {
            builder = builder.allow_any_peer();
        }
        if !config.relay {
            builder = builder.no_relay();
        }
        let agent = builder.build()?;
        let ns = config.namespace.clone();
        let t = |leaf: &str| machine_topic(&ns, leaf);
        Ok(Self {
            info: agent.req_server(&t(topics::MACHINE_INFO))?,
            claim: agent.req_server(&t(topics::MACHINE_CLAIM))?,
            release: agent.req_server(&t(topics::MACHINE_RELEASE))?,
            session_q: agent.req_server(&t(topics::MACHINE_SESSION))?,
            cmd_vel: agent.req_server(&t(topics::MACHINE_CMD_VEL))?,
            cmd: agent.req_server(&t(topics::MACHINE_CMD))?,
            state_pub: agent.publish(&t(topics::MACHINE_STATE))?,
            odom_pub: agent.publish(&t(topics::MACHINE_ODOM))?,
            agent,
            config,
            session: None,
            next_session: 1,
            twist: Twist::zero(),
            commands: Vec::new(),
        })
    }

    pub fn did(&self) -> String {
        self.agent.did_key().unwrap_or_default()
    }

    pub fn addr_hex(&self) -> String {
        crate::registry::endpoint_addr_hex(&self.agent.endpoint_addr())
    }

    pub fn namespace(&self) -> &str {
        &self.config.namespace
    }

    pub fn machine_ref(&self) -> MachineRef {
        let mut r = MachineRef::new(
            &self.config.namespace,
            &self.did(),
            &self.config.kind,
            &self.config.machine_id,
        );
        r.props = Props::from_bytes(&r.props)
            .with("addr", &self.addr_hex())
            .into_bytes();
        r
    }

    pub fn info(&self) -> MachineInfo {
        let mut props = Props::from_pairs(&[
            ("namespace", self.config.namespace.as_str()),
            ("machine_id", self.config.machine_id.as_str()),
            ("kind", self.config.kind.as_str()),
            ("did", &self.did()),
            ("addr", &self.addr_hex()),
        ]);
        for (n, c) in self.config.controllers.iter().enumerate() {
            props.set(&format!("controller.{n}.instance"), &c.instance);
            props.set(&format!("controller.{n}.type"), &c.controller_type);
            if let Some(cmd) = &c.command_interface {
                props.set(&format!("controller.{n}.command_interface"), cmd);
            }
            props.set(
                &format!("controller.{n}.state_interfaces"),
                &c.state_interfaces.join(","),
            );
        }
        MachineInfo {
            controller_count: self.config.controllers.len() as u32,
            held: self.session.is_some() as u32,
            props: props.into_bytes(),
        }
    }

    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    pub fn session_id(&self) -> u64 {
        self.session.as_ref().map(|s| s.id).unwrap_or(0)
    }

    /// The twist the sim should apply right now. Zero once the holder has
    /// been silent longer than its hold time.
    pub fn twist(&self) -> Twist {
        self.twist
    }

    /// Answer every pending request and apply silence rules.
    pub fn poll(&mut self) {
        let info = self.info();
        serve_req(&mut self.info, |_: Ping| info.clone_for_response());

        let now = Instant::now();
        let mut session = self.session.take();
        let mut next_session = self.next_session;
        serve_req(&mut self.claim, |req: ClaimRequest| {
            if let Some(held) = &session
                && req.take == 0
                && now.duration_since(held.last_cmd) < AUTO_RELEASE
            {
                return ClaimResponse::busy(&held.holder);
            }
            let id = next_session;
            next_session += 1;
            let hold = if req.hold_ms == 0 {
                DEFAULT_HOLD
            } else {
                Duration::from_millis(req.hold_ms as u64)
            };
            session = Some(Session {
                id,
                holder: req.client(),
                since: now,
                last_cmd: now,
                hold,
            });
            ClaimResponse::granted(id)
        });
        self.next_session = next_session;

        let mut twist = self.twist;
        serve_req(&mut self.cmd_vel, |req: TwistCmd| match &mut session {
            Some(held) if held.id == req.session => {
                held.last_cmd = now;
                twist = req.twist;
                Status::ok()
            }
            Some(held) => Status::with(
                code::BUSY,
                &[("holder", &held.holder), ("message", "machine is held")],
            ),
            None if req.session == 0 => {
                twist = req.twist;
                Status::ok()
            }
            None => Status::err(code::REFUSED, "no such session; claim first"),
        });

        serve_req(&mut self.release, |req: SessionRef| match &session {
            Some(held) if held.id == req.session || req.session == 0 => {
                session = None;
                twist = Twist::zero();
                Status::ok()
            }
            Some(_) => Status::err(code::REFUSED, "session id does not match holder"),
            None => Status::ok(),
        });

        let mut commands = std::mem::take(&mut self.commands);
        serve_req(&mut self.cmd, |req: ControllerCommand| match &session {
            Some(held) if held.id == req.session => {
                commands.push(req);
                Status::ok()
            }
            None if req.session == 0 => {
                commands.push(req);
                Status::ok()
            }
            _ => Status::err(code::REFUSED, "session id does not match holder"),
        });
        self.commands = commands;

        if let Some(held) = &session {
            let idle = now.duration_since(held.last_cmd);
            if idle > AUTO_RELEASE {
                session = None;
                twist = Twist::zero();
            } else if idle > held.hold {
                twist = Twist::zero();
            }
        }

        let info = SessionInfo {
            session: session.as_ref().map(|s| s.id).unwrap_or(0),
            age_ms: session
                .as_ref()
                .map(|s| now.duration_since(s.since).as_millis() as u64)
                .unwrap_or(0),
            idle_ms: session
                .as_ref()
                .map(|s| now.duration_since(s.last_cmd).as_millis() as u64)
                .unwrap_or(0),
            held: session.is_some() as u32,
            _pad: 0,
            props: Props::from_pairs(&[(
                "holder",
                session.as_ref().map(|s| s.holder.as_str()).unwrap_or(""),
            )])
            .into_bytes(),
        };
        serve_req(&mut self.session_q, |_: Ping| info.clone_for_response());

        self.session = session;
        self.twist = twist;
    }

    pub fn publish_state(&mut self, mut state: MachineState) {
        state.session = self.session_id();
        let _ = self.odom_pub.send(&pack(&state.odom));
        let _ = self.state_pub.send(&pack(&state));
    }

    pub fn clear_session(&mut self) {
        self.session = None;
        self.twist = Twist::zero();
    }
}

trait CloneForResponse {
    fn clone_for_response(&self) -> Self;
}

impl CloneForResponse for MachineInfo {
    fn clone_for_response(&self) -> Self {
        Self {
            controller_count: self.controller_count,
            held: self.held,
            props: self.props.clone(),
        }
    }
}

impl CloneForResponse for SessionInfo {
    fn clone_for_response(&self) -> Self {
        Self {
            session: self.session,
            age_ms: self.age_ms,
            idle_ms: self.idle_ms,
            held: self.held,
            _pad: 0,
            props: self.props.clone(),
        }
    }
}
