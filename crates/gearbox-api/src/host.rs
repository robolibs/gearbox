//! The host agent: one agentio `Agent` per simulator process, hosting the
//! scene-level topics. Servers are polled with closures so a Bevy system
//! can answer from its own queries.

use std::path::PathBuf;
use std::time::Instant;

use agentio::{Agent, DirectoryMode, IdentitySource, Registered};
use datapod::{DataPod, DataPodDecode, DataPodValidate, LeWireHeader};
use peerbus::{AnsServer, EndpointId, Publisher, ReqServer};

use crate::registry::{self, RegistryEntry};
use crate::topics;
use crate::wire::*;

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub instance: String,
    pub ephemeral: bool,
    pub allow: Vec<String>,
    pub allow_any: bool,
    pub relay: bool,
    pub version: String,
    pub log: Option<String>,
    pub write_registry: bool,
}

impl HostConfig {
    pub fn new(instance: &str, version: &str) -> Self {
        Self {
            instance: instance.to_string(),
            ephemeral: false,
            allow: Vec::new(),
            allow_any: false,
            relay: false,
            version: version.to_string(),
            log: None,
            write_registry: true,
        }
    }

    /// Read `GEARBOX_NAME`, `GEARBOX_EPHEMERAL`, `GEARBOX_ALLOW`,
    /// `GEARBOX_ALLOW_ANY`, `GEARBOX_RELAY`, plus the CLI did file.
    pub fn from_env(version: &str) -> Self {
        let mut cfg = Self::new(
            &std::env::var("GEARBOX_NAME").unwrap_or_else(|_| "gearbox".to_string()),
            version,
        );
        cfg.ephemeral = env_flag("GEARBOX_EPHEMERAL");
        cfg.allow_any = env_flag("GEARBOX_ALLOW_ANY");
        cfg.relay = env_flag("GEARBOX_RELAY");
        cfg.log = std::env::var("GEARBOX_LOG").ok().filter(|s| !s.is_empty());
        if let Ok(list) = std::env::var("GEARBOX_ALLOW") {
            cfg.allow.extend(
                list.split([',', ';', ' ', '\n'])
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            );
        }
        if let Some(did) = read_cli_did() {
            cfg.allow.push(did);
        }
        cfg
    }

    pub fn ephemeral(mut self) -> Self {
        self.ephemeral = true;
        self.write_registry = false;
        self
    }
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

pub fn cli_did_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("gearbox").join("cli.did")
}

fn read_cli_did() -> Option<String> {
    let text = std::fs::read_to_string(cli_did_path()).ok()?;
    let did = text.trim();
    (!did.is_empty()).then(|| did.to_string())
}

pub struct HostBus {
    pub agent: Agent,
    pub config: HostConfig,
    started: Instant,
    started_unix_ms: u64,
    info: Registered<ReqServer<Env, Env>>,
    clock: Registered<ReqServer<Env, Env>>,
    clock_state: Registered<Publisher<Env>>,
    clear: Registered<ReqServer<Env, Env>>,
    list: Registered<AnsServer<Env, Env>>,
    events: Registered<Publisher<Env>>,
    usd_load: Registered<ReqServer<Env, Env>>,
    usd_delete: Registered<ReqServer<Env, Env>>,
    marker_set: Registered<ReqServer<Env, Env>>,
    marker_delete: Registered<ReqServer<Env, Env>>,
    select: Registered<ReqServer<Env, Env>>,
    machines: Registered<AnsServer<Env, Env>>,
}

impl HostBus {
    pub fn new(config: HostConfig) -> agentio::Result<Self> {
        let identity = if config.ephemeral {
            IdentitySource::Random
        } else {
            IdentitySource::Name(format!("gearbox/{}", config.instance))
        };
        let mut builder = Agent::builder()
            .name(config.instance.clone())
            .identity(identity)
            .directory(DirectoryMode::Replicated)
            .local_config(shm_limits())
            .allow_peers(config.allow.iter().cloned());
        if config.allow_any {
            builder = builder.allow_any_peer();
        }
        if !config.relay {
            builder = builder.no_relay();
        }
        let agent = builder.build()?;

        let bus = Self {
            info: agent.req_server(topics::HOST_INFO)?,
            clock: agent.req_server(topics::SCENE_CLOCK)?,
            clock_state: agent.publish(topics::SCENE_CLOCK_STATE)?,
            clear: agent.req_server(topics::SCENE_CLEAR)?,
            list: agent.que_server(topics::SCENE_LIST)?,
            events: agent.publish(topics::SCENE_EVENTS)?,
            usd_load: agent.req_server(topics::USD_LOAD)?,
            usd_delete: agent.req_server(topics::USD_DELETE)?,
            marker_set: agent.req_server(topics::MARKER_SET)?,
            marker_delete: agent.req_server(topics::MARKER_DELETE)?,
            select: agent.req_server(topics::SELECT)?,
            machines: agent.que_server(topics::MACHINES_LIST)?,
            agent,
            config,
            started: Instant::now(),
            started_unix_ms: now_unix_ms(),
        };
        if bus.config.write_registry {
            if let Err(err) = registry::write(&bus.registry_entry()) {
                eprintln!("gearbox-api: registry write failed: {err}");
            }
        }
        Ok(bus)
    }

    pub fn did(&self) -> String {
        self.agent.did_key().unwrap_or_default()
    }

    pub fn endpoint_id(&self) -> EndpointId {
        self.agent.endpoint_id()
    }

    pub fn addr(&self) -> String {
        registry::endpoint_addr_hex(&self.agent.endpoint_addr())
    }

    pub fn uptime_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub fn registry_entry(&self) -> RegistryEntry {
        RegistryEntry {
            name: self.config.instance.clone(),
            did: self.did(),
            addr: self.addr(),
            pid: std::process::id(),
            started: registry::now_rfc3339(),
            version: self.config.version.clone(),
            log: self.config.log.clone(),
        }
    }

    pub fn banner(&self) -> String {
        format!(
            "gearbox host `{}`\n  did      {}\n  registry {}\n  allowed  {} peer(s), any: {}",
            self.config.instance,
            self.did(),
            if self.config.write_registry {
                registry::entry_path(&self.config.instance)
                    .display()
                    .to_string()
            } else {
                "(ephemeral, none)".to_string()
            },
            self.config.allow.len(),
            self.config.allow_any,
        )
    }

    pub fn serve_info(&mut self, f: impl FnMut(Ping) -> HostInfo) -> usize {
        serve_req(&mut self.info, f)
    }

    pub fn serve_clock(&mut self, f: impl FnMut(ClockCommand) -> ClockState) -> usize {
        serve_req(&mut self.clock, f)
    }

    pub fn serve_clear(&mut self, f: impl FnMut(ClearRequest) -> Status) -> usize {
        serve_req(&mut self.clear, f)
    }

    pub fn serve_list(&mut self, f: impl FnMut(ListQuery) -> Vec<SceneObject>) -> usize {
        serve_que(&mut self.list, f)
    }

    pub fn serve_usd_load(&mut self, mut f: impl FnMut(UsdLoad) -> Status) -> usize {
        let started = self.started_unix_ms;
        serve_req(&mut self.usd_load, |req: UsdLoad| {
            if predates(&req.props, started) {
                stale_status("load", &req.id())
            } else {
                f(req)
            }
        })
    }

    pub fn serve_usd_delete(&mut self, mut f: impl FnMut(UsdRef) -> Status) -> usize {
        let started = self.started_unix_ms;
        serve_req(&mut self.usd_delete, |req: UsdRef| {
            if predates(&req.props, started) {
                stale_status("delete", &req.id())
            } else {
                f(req)
            }
        })
    }

    pub fn serve_marker_set(&mut self, f: impl FnMut(MarkerSet) -> Status) -> usize {
        serve_req(&mut self.marker_set, f)
    }

    pub fn serve_marker_delete(&mut self, f: impl FnMut(MarkerRef) -> Status) -> usize {
        serve_req(&mut self.marker_delete, f)
    }

    pub fn serve_select(&mut self, f: impl FnMut(Selection) -> Selection) -> usize {
        serve_req(&mut self.select, f)
    }

    pub fn serve_machines(&mut self, f: impl FnMut(Ping) -> Vec<MachineRef>) -> usize {
        serve_que(&mut self.machines, f)
    }

    pub fn publish_event(&mut self, event: &SceneEvent) {
        if let Err(err) = self.events.send(&pack(event)) {
            eprintln!("gearbox-api: scene event publish failed: {err}");
        }
    }

    pub fn publish_clock(&mut self, state: &ClockState) {
        let _ = self.clock_state.send(&pack(state));
    }
}

impl Drop for HostBus {
    fn drop(&mut self) {
        if self.config.write_registry {
            registry::remove(&self.config.instance);
        }
    }
}

/// Shared-memory limits for every gearbox agent: many local clients and
/// watchers per topic, small messages, and a ring deep enough to absorb a
/// burst (a field of props settling at once) while a script sleeps.
pub fn shm_limits() -> agentio::LocalConfig {
    agentio::LocalConfig {
        max_publishers: 8,
        max_subscribers: 16,
        subscriber_buffer: 512,
        history_depth: 1,
        max_payload_bytes: 16 * 1024,
    }
}

/// Drain every pending request on a req/res server, answering each with
/// the closure. Undecodable requests are dropped.
/// Requests left over from a client's session with an earlier instance of
/// this host arrive again when the host restarts; the `sent_at` stamp tells
/// them apart from live ones. Unstamped requests pass.
const STALE_SKEW_MS: u64 = 2_000;

fn predates(props: &[u8], started_unix_ms: u64) -> bool {
    Props::from_bytes(props)
        .get(SENT_AT)
        .and_then(|s| s.parse::<u64>().ok())
        .is_some_and(|sent| sent + STALE_SKEW_MS < started_unix_ms)
}

fn stale_status(what: &str, id: &str) -> Status {
    eprintln!("gearbox-api: dropped {what} `{id}` sent before this instance started");
    Status::err(code::REFUSED, "request predates this instance")
}

pub fn serve_req<Req, Res>(server: &mut ReqServer<Env, Env>, mut f: impl FnMut(Req) -> Res) -> usize
where
    Req: DataPodDecode + DataPodValidate + 'static,
    Req::Header: LeWireHeader,
    Res: DataPod + DataPodValidate + 'static,
    Res::Header: LeWireHeader,
{
    let mut served = 0;
    loop {
        let (sample, reply) = match server.take() {
            Ok(Some(pending)) => pending,
            _ => break,
        };
        match unpack::<Req>(sample.header().type_hash, sample.payload()) {
            Ok(req) => {
                let res = pack(&f(req));
                if reply.respond(&res).is_ok() {
                    served += 1;
                }
            }
            Err(err) => eprintln!("gearbox-api: undecodable request dropped: {err}"),
        }
    }
    served
}

/// Drain every pending query on a que/ans server, streaming the closure's
/// answers and finishing each.
pub fn serve_que<Que, Ans>(
    server: &mut AnsServer<Env, Env>,
    mut f: impl FnMut(Que) -> Vec<Ans>,
) -> usize
where
    Que: DataPodDecode + DataPodValidate + 'static,
    Que::Header: LeWireHeader,
    Ans: DataPod + DataPodValidate + 'static,
    Ans::Header: LeWireHeader,
{
    let mut served = 0;
    loop {
        let pending = match server.take_message() {
            Ok(Some(pending)) => pending,
            _ => break,
        };
        let (sample, token) = pending.into_parts();
        let answers = match unpack::<Que>(sample.header().type_hash, sample.payload()) {
            Ok(que) => f(que),
            Err(err) => {
                eprintln!("gearbox-api: undecodable query dropped: {err}");
                Vec::new()
            }
        };
        for ans in &answers {
            if server.send_pending(token, &pack(ans)).is_err() {
                break;
            }
        }
        let _ = server.finish_pending(token);
        served += 1;
    }
    served
}
