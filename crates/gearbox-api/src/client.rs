//! Typed client for a running host and its machines. Used by tests, the
//! CLI, and anything else written in Rust.

use std::time::{Duration, Instant};

use agentio::{Agent, DirectoryMode, IdentitySource};
use datapod::robot::{Imu, Odom, TurnRadius, WheelEncoders};
use datapod::{DataPod, DataPodDecode, DataPodValidate, LeWireHeader};
use peerbus::{EndpointId, ReqClient, Subscriber};

use crate::topics::{self, machine_topic};
use crate::wire::*;

pub struct Client {
    pub agent: Agent,
    pub host: EndpointId,
}

impl Client {
    /// Connect to a host by `did:key` or raw id, with the given identity.
    pub fn connect(host_did: &str, identity: IdentitySource, name: &str) -> agentio::Result<Self> {
        let host = agentio::did_key::did_key_to_endpoint(host_did)?;
        Self::connect_id(host, identity, name)
    }

    pub fn connect_id(
        host: EndpointId,
        identity: IdentitySource,
        name: &str,
    ) -> agentio::Result<Self> {
        let agent = Agent::builder()
            .name(name)
            .identity(identity)
            .bootstrap([host])
            .directory(DirectoryMode::FrontDoor(host))
            .local_config(crate::host::shm_limits())
            .allow_peer(host)
            .no_relay()
            .build()?;
        Ok(Self { agent, host })
    }

    pub fn did(&self) -> String {
        self.agent.did_key().unwrap_or_default()
    }

    fn call<Req, Res>(&self, topic: &str, req: &Req) -> agentio::Result<Res>
    where
        Req: DataPod + DataPodValidate + 'static,
        Req::Header: LeWireHeader,
        Res: DataPodDecode + DataPodValidate + 'static,
        Res::Header: LeWireHeader,
    {
        let mut client: ReqClient<Env, Env> = resolve_retry(|| self.agent.req_client(topic))?;
        call_with(&mut client, req)
    }

    fn query<Que, Ans>(&self, topic: &str, que: &Que) -> agentio::Result<Vec<Ans>>
    where
        Que: DataPod + DataPodValidate + 'static,
        Que::Header: LeWireHeader,
        Ans: DataPodDecode + DataPodValidate + 'static,
        Ans::Header: LeWireHeader,
    {
        let mut client = resolve_retry(|| self.agent.que_client::<Env, Env>(topic))?;
        let mut answers = client.send(&pack(que))?;
        let mut out = Vec::new();
        while let Some(ans) = answers.next()? {
            out.push(unpack::<Ans>(ans.header().type_hash, ans.payload()).map_err(wire_err)?);
        }
        Ok(out)
    }

    fn subscribe(&self, topic: &str) -> agentio::Result<Subscriber<Env>> {
        resolve_retry(|| self.agent.subscribe::<Env>(topic))
    }

    /// Untyped req/res: the caller builds and reads envelopes itself.
    pub fn call_env(&self, topic: &str, req: &Env) -> agentio::Result<Env> {
        let mut client: ReqClient<Env, Env> = resolve_retry(|| self.agent.req_client(topic))?;
        let res = client.call(req)?;
        Ok(Env::new(res.header().type_hash, res.payload().to_vec()))
    }

    /// Untyped que/ans: every answer as an envelope.
    pub fn query_env(&self, topic: &str, que: &Env) -> agentio::Result<Vec<Env>> {
        let mut client = resolve_retry(|| self.agent.que_client::<Env, Env>(topic))?;
        let mut answers = client.send(que)?;
        let mut out = Vec::new();
        while let Some(ans) = answers.next()? {
            out.push(Env::new(ans.header().type_hash, ans.payload().to_vec()));
        }
        Ok(out)
    }

    pub fn subscribe_env(&self, topic: &str) -> agentio::Result<Subscriber<Env>> {
        self.subscribe(topic)
    }

    pub fn resolve_topic(&self, topic: &str) -> agentio::Result<agentio::TopicEntry> {
        resolve_retry(|| self.agent.resolve_topic(topic))
    }

    pub fn path_diagnostics(
        &self,
        peer: EndpointId,
    ) -> agentio::Result<Option<peerbus::PeerPathDiagnostics>> {
        Ok(self.agent.node().peer_path_diagnostics(peer)?)
    }

    pub fn info(&self) -> agentio::Result<HostInfo> {
        self.call(topics::HOST_INFO, &Ping::default())
    }

    pub fn clock(&self, op: u32) -> agentio::Result<ClockState> {
        self.call(topics::SCENE_CLOCK, &ClockCommand { op, steps: 0 })
    }

    pub fn clear(&self, scope: u32) -> agentio::Result<Status> {
        self.call(
            topics::SCENE_CLEAR,
            &ClearRequest {
                scope,
                pause_clock: 0,
            },
        )
    }

    pub fn list(&self, kind: u32) -> agentio::Result<Vec<SceneObject>> {
        self.query(topics::SCENE_LIST, &ListQuery { kind })
    }

    pub fn load(&self, req: &UsdLoad) -> agentio::Result<Status> {
        let stamped = req.clone().with_prop(SENT_AT, &now_unix_ms().to_string());
        self.call(topics::USD_LOAD, &stamped)
    }

    pub fn delete(&self, id: &str) -> agentio::Result<Status> {
        let req = UsdRef::new(id).with_prop(SENT_AT, &now_unix_ms().to_string());
        self.call(topics::USD_DELETE, &req)
    }

    pub fn marker_set(&self, id: &str, x: f32, y: f32, z: f32) -> agentio::Result<Status> {
        self.call(topics::MARKER_SET, &MarkerSet::new(id, x, y, z))
    }

    pub fn marker_delete(&self, id: &str) -> agentio::Result<Status> {
        self.call(topics::MARKER_DELETE, &MarkerRef::new(id))
    }

    pub fn select(&self, req: &Selection) -> agentio::Result<Selection> {
        self.call(topics::SELECT, req)
    }

    pub fn machines(&self) -> agentio::Result<Vec<MachineRef>> {
        self.query(topics::MACHINES_LIST, &Ping::default())
    }

    pub fn events(&self) -> agentio::Result<Subscriber<Env>> {
        self.subscribe(topics::SCENE_EVENTS)
    }

    pub fn clock_state(&self) -> agentio::Result<Subscriber<Env>> {
        self.subscribe(topics::SCENE_CLOCK_STATE)
    }

    pub fn machine(&self, namespace: &str) -> MachineClient<'_> {
        MachineClient {
            client: self,
            namespace: namespace.to_string(),
            cmd_vel: None,
        }
    }

    /// Wait until the host answers `info`, for scripts racing a launch.
    pub fn wait_ready(&self, timeout: Duration) -> agentio::Result<HostInfo> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.info() {
                Ok(info) => return Ok(info),
                Err(err) if std::time::Instant::now() >= deadline => return Err(err),
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }
}

pub struct MachineClient<'a> {
    client: &'a Client,
    namespace: String,
    cmd_vel: Option<ReqClient<Env, Env>>,
}

impl MachineClient<'_> {
    fn topic(&self, leaf: &str) -> String {
        machine_topic(&self.namespace, leaf)
    }

    pub fn info(&self) -> agentio::Result<MachineInfo> {
        self.client
            .call(&self.topic(topics::MACHINE_INFO), &Ping::default())
    }

    pub fn claim(&self, hold_ms: u32, take: bool) -> agentio::Result<ClaimResponse> {
        let mut req = ClaimRequest::new(&self.client.did(), hold_ms);
        if take {
            req = req.taking();
        }
        self.client.call(&self.topic(topics::MACHINE_CLAIM), &req)
    }

    pub fn release(&self, session: u64) -> agentio::Result<Status> {
        self.client.call(
            &self.topic(topics::MACHINE_RELEASE),
            &SessionRef { session },
        )
    }

    pub fn session(&self) -> agentio::Result<SessionInfo> {
        self.client
            .call(&self.topic(topics::MACHINE_SESSION), &Ping::default())
    }

    /// Cached client, since this is called at command rate.
    pub fn cmd_vel(
        &mut self,
        session: u64,
        forward_mps: f64,
        yaw_rps: f64,
    ) -> agentio::Result<Status> {
        if self.cmd_vel.is_none() {
            let topic = self.topic(topics::MACHINE_CMD_VEL);
            self.cmd_vel = Some(resolve_retry(|| self.client.agent.req_client(&topic))?);
        }
        let client = self.cmd_vel.as_mut().expect("cached above");
        call_with(client, &TwistCmd::new(session, forward_mps, yaw_rps))
    }

    pub fn command(&self, cmd: &ControllerCommand) -> agentio::Result<Status> {
        self.client.call(&self.topic(topics::MACHINE_CMD), cmd)
    }

    pub fn state(&self) -> agentio::Result<Subscriber<Env>> {
        self.client.subscribe(&self.topic(topics::MACHINE_STATE))
    }

    pub fn odom(&self) -> agentio::Result<Subscriber<Env>> {
        self.client.subscribe(&self.topic(topics::MACHINE_ODOM))
    }

    /// Link poses, once `set_tf(true)` has switched the stream on.
    pub fn tf(&self) -> agentio::Result<Subscriber<Env>> {
        self.client.subscribe(&self.topic(topics::MACHINE_TF))
    }

    pub fn encoders(&self) -> agentio::Result<Subscriber<Env>> {
        self.client.subscribe(&self.topic(topics::MACHINE_ENCODERS))
    }

    pub fn imu(&self) -> agentio::Result<Subscriber<Env>> {
        self.client.subscribe(&self.topic(topics::MACHINE_IMU))
    }

    pub fn turn_radius(&self) -> agentio::Result<Subscriber<Env>> {
        self.client
            .subscribe(&self.topic(topics::MACHINE_TURN_RADIUS))
    }

    pub fn set_tf(&self, on: bool) -> agentio::Result<Status> {
        let cmd = ControllerCommand {
            props: Props::from_pairs(&[("tf", if on { "on" } else { "off" })]).into_bytes(),
            ..Default::default()
        };
        self.command(&cmd)
    }

    /// Hang `slave` on one of this machine's hitches.
    pub fn attach(&self, req: &AttachRequest) -> agentio::Result<Status> {
        self.client
            .call(&self.topic(topics::MACHINE_TOOLS_ATTACH), req)
    }

    pub fn detach(&self, session: u64, slave: &str) -> agentio::Result<Status> {
        self.client.call(
            &self.topic(topics::MACHINE_TOOLS_DETACH),
            &DetachRequest::new(session, slave),
        )
    }

    /// Attachments below this machine, depth-first.
    pub fn tools(&self) -> agentio::Result<Vec<AttachmentRecord>> {
        self.client
            .query(&self.topic(topics::MACHINE_TOOLS), &Ping::default())
    }

    /// The link tree, base_link first.
    pub fn links(&self) -> agentio::Result<Vec<LinkRecord>> {
        self.client
            .query(&self.topic(topics::MACHINE_LINKS), &Ping::default())
    }

    pub fn next_odom(
        &self,
        sub: &mut Subscriber<Env>,
        timeout: Duration,
    ) -> agentio::Result<Option<Odom>> {
        next_sample::<Odom>(sub, timeout)
    }

    pub fn next_encoders(
        &self,
        sub: &mut Subscriber<Env>,
        timeout: Duration,
    ) -> agentio::Result<Option<WheelEncoders>> {
        next_sample::<WheelEncoders>(sub, timeout)
    }

    pub fn next_imu(
        &self,
        sub: &mut Subscriber<Env>,
        timeout: Duration,
    ) -> agentio::Result<Option<Imu>> {
        next_sample::<Imu>(sub, timeout)
    }

    pub fn next_turn_radius(
        &self,
        sub: &mut Subscriber<Env>,
        timeout: Duration,
    ) -> agentio::Result<Option<TurnRadius>> {
        next_sample::<TurnRadius>(sub, timeout)
    }
}

/// Directory queries can fail while a fresh agent's control worker is
/// still connecting, so retry resolution for a short while.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

fn resolve_retry<T>(mut f: impl FnMut() -> agentio::Result<T>) -> agentio::Result<T> {
    let deadline = std::time::Instant::now() + RESOLVE_TIMEOUT;
    loop {
        match f() {
            Ok(v) => return Ok(v),
            Err(err) => {
                let retry = matches!(
                    err,
                    agentio::Error::ResolutionFailed(_) | agentio::Error::ControlTimeout(_)
                );
                if !retry || std::time::Instant::now() >= deadline {
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn call_with<Req, Res>(client: &mut ReqClient<Env, Env>, req: &Req) -> agentio::Result<Res>
where
    Req: DataPod + DataPodValidate + 'static,
    Req::Header: LeWireHeader,
    Res: DataPodDecode + DataPodValidate + 'static,
    Res::Header: LeWireHeader,
{
    let res = client.call(&pack(req))?;
    unpack::<Res>(res.header().type_hash, res.payload()).map_err(wire_err)
}

fn wire_err(err: datapod::WireError) -> agentio::Error {
    agentio::Error::Format(err.to_string())
}

/// Poll an envelope subscriber until a sample arrives or the timeout
/// passes, decoding it as `T`.
pub fn next_sample<T>(sub: &mut Subscriber<Env>, timeout: Duration) -> agentio::Result<Option<T>>
where
    T: DataPodDecode + DataPodValidate + 'static,
    T::Header: LeWireHeader,
{
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match sub.recv_timeout(remaining) {
            Ok(Some(sample)) => {
                return Ok(Some(
                    unpack::<T>(sample.header().type_hash, sample.payload()).map_err(wire_err)?,
                ));
            }
            Ok(None) => return Ok(None),
            // A slow reader skips the samples it missed and carries on.
            Err(peerbus::Error::Lagged { .. }) => continue,
            Err(err) => return Err(err.into()),
        }
    }
}
