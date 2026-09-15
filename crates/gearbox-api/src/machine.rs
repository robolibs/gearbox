//! A machine agent: one agentio `Agent` per simulated machine, hosting the
//! control and telemetry topics for that machine under its own identity.

use std::time::{Duration, Instant};

use agentio::{Agent, DirectoryMode, IdentitySource, Registered};
use datapod::robot::{Imu, Twist, TurnRadius, WheelEncoders};
use peerbus::{AnsServer, EndpointId, Publisher, ReqReplyToken, ReqServer};

use crate::host::{serve_que, serve_req};
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
    /// The link tree, base_link first; `derived` when the asset marked none.
    pub links: Vec<LinkDesc>,
    pub links_derived: bool,
    pub ephemeral: bool,
    pub allow: Vec<String>,
    pub allow_any: bool,
    pub relay: bool,
}

/// One link as the agent answers it on `/links`.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkDesc {
    pub name: String,
    pub parent: Option<String>,
    pub role: String,
    pub prim: String,
    pub joint: Option<String>,
    pub body: Option<String>,
    /// Static offset to the parent: translation then quaternion (w, x, y, z).
    pub offset: [f64; 7],
    /// `side|type|name` of a coupling on this link, when it carries one.
    pub coupling: Option<String>,
    /// Element kind when the link is a working part: function, bin,
    /// section, unit, connector, navigation.
    pub element: Option<String>,
    pub number: Option<u32>,
    pub designator: Option<String>,
    /// Named values authored on the link (`gearbox:value:<Name>`).
    pub values: Vec<(String, f64)>,
}

impl LinkDesc {
    pub fn new(name: &str, parent: Option<&str>, role: &str) -> Self {
        Self {
            name: name.to_string(),
            parent: parent.map(str::to_string),
            role: role.to_string(),
            prim: String::new(),
            joint: None,
            body: None,
            offset: [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            coupling: None,
            element: None,
            number: None,
            designator: None,
            values: Vec::new(),
        }
    }
}

impl MachineConfig {
    pub fn new(instance: &str, namespace: &str) -> Self {
        Self {
            instance: instance.to_string(),
            namespace: namespace.to_string(),
            machine_id: namespace.to_string(),
            kind: String::new(),
            controllers: Vec::new(),
            links: Vec::new(),
            links_derived: false,
            ephemeral: false,
            allow: Vec::new(),
            allow_any: false,
            relay: false,
        }
    }

    /// The smallest valid tree: a base and one wheel, for fakes and tests.
    pub fn with_minimal_links(mut self) -> Self {
        self.links = vec![
            LinkDesc::new("base_link", None, "base"),
            LinkDesc::new("wheel_left", Some("base_link"), "wheel"),
        ];
        self.links_derived = true;
        self
    }

    pub fn link_records(&self) -> Vec<LinkRecord> {
        link_records_for(&self.links)
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
    links: Registered<AnsServer<Env, Env>>,
    tf_pub: Registered<Publisher<Env>>,
    tf_enabled: bool,
    encoders_pub: Registered<Publisher<Env>>,
    imu_pub: Registered<Publisher<Env>>,
    turn_radius_pub: Registered<Publisher<Env>>,
    attach: Registered<ReqServer<Env, Env>>,
    detach: Registered<ReqServer<Env, Env>>,
    tools_q: Registered<AnsServer<Env, Env>>,
    /// Set while this machine hangs on another machine's hitch.
    attached_to: Option<String>,
    /// Slaves hanging on this machine, depth-first, and their re-parented
    /// links appended to `/links`.
    tools: Vec<ToolDesc>,
    tool_links: Vec<LinkDesc>,
    /// Attach and detach requests waiting for the sim to act and answer.
    pub pending_attach: Vec<(AttachRequest, ReqReplyToken)>,
    pub pending_detach: Vec<(DetachRequest, ReqReplyToken)>,
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
            links: agent.que_server(&t(topics::MACHINE_LINKS))?,
            tf_pub: agent.publish(&t(topics::MACHINE_TF))?,
            tf_enabled: false,
            encoders_pub: agent.publish(&t(topics::MACHINE_ENCODERS))?,
            imu_pub: agent.publish(&t(topics::MACHINE_IMU))?,
            turn_radius_pub: agent.publish(&t(topics::MACHINE_TURN_RADIUS))?,
            attach: agent.req_server(&t(topics::MACHINE_TOOLS_ATTACH))?,
            detach: agent.req_server(&t(topics::MACHINE_TOOLS_DETACH))?,
            tools_q: agent.que_server(&t(topics::MACHINE_TOOLS))?,
            attached_to: None,
            tools: Vec::new(),
            tool_links: Vec::new(),
            pending_attach: Vec::new(),
            pending_detach: Vec::new(),
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
            ("link_count", &self.config.links.len().to_string()),
            ("links_derived", &self.config.links_derived.to_string()),
            ("attached_to", self.attached_to.as_deref().unwrap_or("")),
            (
                "tools",
                &self
                    .tools
                    .iter()
                    .map(|t| t.slave.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            (
                "base_link",
                self.config
                    .links
                    .iter()
                    .find(|l| l.role == "base")
                    .map(|l| l.prim.as_str())
                    .unwrap_or(""),
            ),
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
        let all_links: Vec<LinkDesc> = self
            .config
            .links
            .iter()
            .chain(self.tool_links.iter())
            .cloned()
            .collect();
        serve_que(&mut self.links, |_: Ping| link_records_for(&all_links));
        let master = self.config.namespace.clone();
        let tools = &self.tools;
        serve_que(&mut self.tools_q, |_: Ping| {
            tools.iter().map(|t| t.record(&master)).collect()
        });
        while let Ok(Some(pending)) = self.attach.take_message() {
            let (sample, token) = pending.into_parts();
            match unpack::<AttachRequest>(sample.header().type_hash, sample.payload()) {
                Ok(req) => self.pending_attach.push((req, token)),
                Err(err) => {
                    let _ = self
                        .attach
                        .respond_pending(token, &pack(&Status::err(code::USAGE, &err.to_string())));
                }
            }
        }
        while let Ok(Some(pending)) = self.detach.take_message() {
            let (sample, token) = pending.into_parts();
            match unpack::<DetachRequest>(sample.header().type_hash, sample.payload()) {
                Ok(req) => self.pending_detach.push((req, token)),
                Err(err) => {
                    let _ = self
                        .detach
                        .respond_pending(token, &pack(&Status::err(code::USAGE, &err.to_string())));
                }
            }
        }
        let attached_to = self.attached_to.clone();

        let now = Instant::now();
        let mut session = self.session.take();
        let mut next_session = self.next_session;
        serve_req(&mut self.claim, |req: ClaimRequest| {
            if let Some(master) = &attached_to {
                return ClaimResponse {
                    session: 0,
                    code: code::REFUSED,
                    _pad: 0,
                    props: Props::from_pairs(&[
                        ("attached_to", master.as_str()),
                        (
                            "message",
                            "machine is attached; command it through its master",
                        ),
                    ])
                    .into_bytes(),
                };
            }
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
        serve_req(&mut self.cmd_vel, |req: TwistCmd| {
            if let Some(master) = &attached_to {
                return Status::with(
                    code::REFUSED,
                    &[
                        ("attached_to", master.as_str()),
                        (
                            "message",
                            "machine is attached; command it through its master",
                        ),
                    ],
                );
            }
            match &mut session {
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
            }
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
        let mut tf_enabled = self.tf_enabled;
        serve_req(&mut self.cmd, |req: ControllerCommand| {
            // Switching the tf stream is a read-only concern; no session needed.
            if let Some(flag) = req.props().get("tf") {
                tf_enabled = matches!(flag.as_str(), "on" | "1" | "true");
                return Status::ok();
            }
            match &session {
                Some(held) if held.id == req.session => {
                    commands.push(req);
                    Status::ok()
                }
                None if req.session == 0 => {
                    commands.push(req);
                    Status::ok()
                }
                _ => Status::err(code::REFUSED, "session id does not match holder"),
            }
        });
        self.commands = commands;
        self.tf_enabled = tf_enabled;

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

    pub fn tf_enabled(&self) -> bool {
        self.tf_enabled
    }

    pub fn publish_link_pose(&mut self, pose: &LinkPose) {
        let _ = self.tf_pub.send(&pack(pose));
    }

    pub fn attached_to(&self) -> Option<&str> {
        self.attached_to.as_deref()
    }

    pub fn set_attached_to(&mut self, master: Option<String>) {
        self.attached_to = master;
        if self.attached_to.is_some() {
            self.clear_session();
        }
    }

    pub fn tools(&self) -> &[ToolDesc] {
        &self.tools
    }

    /// Replace the composite below this machine: its attachments and the
    /// slaves' links re-parented under the hitch links.
    pub fn set_tools(&mut self, tools: Vec<ToolDesc>, tool_links: Vec<LinkDesc>) {
        self.tools = tools;
        self.tool_links = tool_links;
    }

    pub fn respond_attach(&mut self, token: ReqReplyToken, status: &Status) {
        let _ = self.attach.respond_pending(token, &pack(status));
    }

    pub fn respond_detach(&mut self, token: ReqReplyToken, status: &Status) {
        let _ = self.detach.respond_pending(token, &pack(status));
    }

    pub fn publish_state(&mut self, mut state: MachineState) {
        state.session = self.session_id();
        let _ = self.odom_pub.send(&pack(&state.odom));
        let _ = self.state_pub.send(&pack(&state));
    }

    pub fn publish_encoders(&mut self, encoders: &WheelEncoders) {
        let _ = self.encoders_pub.send(&pack(encoders));
    }

    pub fn publish_imu(&mut self, imu: &Imu) {
        let _ = self.imu_pub.send(&pack(imu));
    }

    pub fn publish_turn_radius(&mut self, turn_radius: &TurnRadius) {
        let _ = self.turn_radius_pub.send(&pack(turn_radius));
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

/// One slave hanging on a hitch of this machine.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDesc {
    pub slave: String,
    pub hitch: String,
    pub coupler: String,
    pub kind: String,
    pub controlled: bool,
    pub depth: u32,
    /// Requests of the slave the master does not grant, comma-separated.
    pub denied: String,
}

impl ToolDesc {
    pub fn record(&self, master: &str) -> AttachmentRecord {
        AttachmentRecord {
            controlled: self.controlled as u32,
            depth: self.depth,
            props: Props::from_pairs(&[
                ("master", master),
                ("slave", self.slave.as_str()),
                ("hitch", self.hitch.as_str()),
                ("coupler", self.coupler.as_str()),
                ("type", self.kind.as_str()),
                ("denied", self.denied.as_str()),
            ])
            .into_bytes(),
        }
    }
}

/// Records for a link list, parents resolved by name within the list.
pub fn link_records_for(links: &[LinkDesc]) -> Vec<LinkRecord> {
    let index_of = |name: &str| {
        links
            .iter()
            .position(|l| l.name == name)
            .map(|i| i as u32)
            .unwrap_or(LinkRecord::NO_PARENT)
    };
    links
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut props = Props::from_pairs(&[
                ("name", l.name.as_str()),
                ("role", l.role.as_str()),
                ("prim", l.prim.as_str()),
            ]);
            if let Some(p) = &l.parent {
                props.set("parent", p);
            }
            if let Some(j) = &l.joint {
                props.set("joint", j);
            }
            if let Some(b) = &l.body {
                props.set("body", b);
            }
            if let Some(c) = &l.coupling {
                props.set("coupling", c);
            }
            if let Some(e) = &l.element {
                props.set("element", e);
            }
            if let Some(n) = l.number {
                props.set("number", &n.to_string());
            }
            if let Some(d) = &l.designator {
                props.set("designator", d);
            }
            for (k, v) in &l.values {
                props.set(&format!("value.{k}"), &v.to_string());
            }
            LinkRecord {
                x: l.offset[0],
                y: l.offset[1],
                z: l.offset[2],
                qw: l.offset[3],
                qx: l.offset[4],
                qy: l.offset[5],
                qz: l.offset[6],
                index: i as u32,
                parent_index: l
                    .parent
                    .as_deref()
                    .map(index_of)
                    .unwrap_or(LinkRecord::NO_PARENT),
                props: props.into_bytes(),
            }
        })
        .collect()
}
