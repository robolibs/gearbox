//! An in-process fake host with canned scene state and kinematic fake
//! machines, so clients and the CLI can be tested without Bevy or a GPU.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use datapod::robot::Odom;
use datapod::{Point, Quaternion};
use peerbus::EndpointId;

use crate::host::{HostBus, HostConfig};
use crate::machine::{MachineAgent, MachineConfig};
use crate::wire::*;

#[derive(Debug, Clone, Default)]
pub struct FakeScene {
    pub objects: Vec<SceneObject>,
    pub markers: Vec<(String, [f32; 3])>,
    pub paused: bool,
    pub selection: Option<(u32, String)>,
    pub cleared: usize,
}

pub struct FakeHost {
    pub did: String,
    pub addr_hex: String,
    pub endpoint_id: EndpointId,
    pub scene: Arc<Mutex<FakeScene>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeHost {
    pub fn start(instance: &str, machines: &[&str]) -> agentio::Result<Self> {
        let config = HostConfig::new(instance, "fake").ephemeral();
        let mut host = HostBus::new(config)?;
        let did = host.did();
        let addr_hex = host.addr();
        let endpoint_id = host.endpoint_id();
        let mut agents = Vec::new();
        for ns in machines {
            let cfg = MachineConfig::new(instance, ns);
            let mut cfg = cfg.with_cmd_vel("drive", "builtin:ackermann_cmd_vel");
            cfg.ephemeral = true;
            cfg.kind = "fake".to_string();
            agents.push(FakeMachine {
                agent: MachineAgent::new(cfg, endpoint_id)?,
                x: 0.0,
                y: 0.0,
                heading: 0.0,
                last: Instant::now(),
            });
        }
        let scene = Arc::new(Mutex::new(FakeScene::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let instance = instance.to_string();
        let thread = {
            let scene = scene.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut last_state = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    let spawn = serve_once(&mut host, &scene, &mut agents);
                    for ns in spawn {
                        if agents.iter().any(|m| m.agent.namespace() == ns) {
                            continue;
                        }
                        let mut cfg = MachineConfig::new(&instance, &ns)
                            .with_cmd_vel("drive", "builtin:ackermann_cmd_vel");
                        cfg.ephemeral = true;
                        cfg.kind = "fake".to_string();
                        if let Ok(agent) = MachineAgent::new(cfg, endpoint_id) {
                            let (x, z, heading) = scene
                                .lock()
                                .ok()
                                .and_then(|s| {
                                    s.objects
                                        .iter()
                                        .find(|o| {
                                            o.props().get("namespace").as_deref()
                                                == Some(ns.as_str())
                                        })
                                        .map(|o| {
                                            (
                                                o.x as f64,
                                                o.z as f64,
                                                (o.yaw_deg as f64).to_radians(),
                                            )
                                        })
                                })
                                .unwrap_or((0.0, 0.0, 0.0));
                            agents.push(FakeMachine {
                                agent,
                                x,
                                y: z,
                                heading,
                                last: Instant::now(),
                            });
                        }
                    }
                    if last_state.elapsed() >= Duration::from_millis(50) {
                        for m in &mut agents {
                            m.step();
                        }
                        harvest_touched_bales(&mut host, &scene, &agents);
                        last_state = Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        };
        Ok(Self {
            did,
            addr_hex,
            endpoint_id,
            scene,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for FakeHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct FakeMachine {
    agent: MachineAgent,
    x: f64,
    y: f64,
    heading: f64,
    last: Instant,
}

impl FakeMachine {
    fn step(&mut self) {
        let dt = self.last.elapsed().as_secs_f64();
        self.last = Instant::now();
        // Gearbox heading convention: 0 faces +Z, +π/2 faces +X. The real
        // controller keeps the steer angle's sign when reversing, so the body
        // turns the other way; mirror that here.
        let twist = self.agent.twist();
        let yaw = if twist.linear.vx < 0.0 {
            -twist.angular.vz
        } else {
            twist.angular.vz
        };
        self.heading += yaw * dt;
        self.x += twist.linear.vx * self.heading.sin() * dt;
        self.y += twist.linear.vx * self.heading.cos() * dt;
        let half = self.heading * 0.5;
        let state = MachineState {
            odom: Odom {
                pose: datapod::Pose {
                    point: Point::new(self.x, 0.0, self.y),
                    rotation: Quaternion::new(half.cos(), 0.0, half.sin(), 0.0),
                },
                twist,
            },
            heading_rad: self.heading,
            roll_rad: 0.0,
            pitch_rad: 0.0,
            session: 0,
            props: Props::from_pairs(&[("controller", "drive")]).into_bytes(),
        };
        self.agent.publish_state(state);
    }
}

/// A machine within this distance of a `bale_*` prop collects it, as the
/// real world does on contact.
const FAKE_HARVEST_RADIUS_M: f64 = 3.0;

fn harvest_touched_bales(
    host: &mut HostBus,
    scene: &Arc<Mutex<FakeScene>>,
    agents: &[FakeMachine],
) {
    let Ok(mut s) = scene.lock() else { return };
    let mut events = Vec::new();
    s.objects.retain(|o| {
        let id = o.props().get("id").unwrap_or_default();
        if !id.starts_with("bale_") {
            return true;
        }
        let touched = agents.iter().any(|m| {
            let dx = m.x - o.x as f64;
            let dz = m.y - o.z as f64;
            (dx * dx + dz * dz).sqrt() < FAKE_HARVEST_RADIUS_M
        });
        if touched {
            let bale_id = id
                .trim_start_matches("bale_")
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>();
            events.push(
                SceneEvent::new(event_kind::HARVESTED, &id)
                    .at(o.x, o.y, o.z)
                    .with_prop("bale_id", &bale_id),
            );
        }
        !touched
    });
    drop(s);
    for ev in &events {
        host.publish_event(ev);
    }
}

/// Serve one round of host requests. Returns namespaces of machine loads
/// that arrived, so the caller can create their agents.
fn serve_once(
    host: &mut HostBus,
    scene: &Arc<Mutex<FakeScene>>,
    agents: &mut [FakeMachine],
) -> Vec<String> {
    let mut spawn = Vec::new();
    let Ok(mut s) = scene.lock() else {
        return spawn;
    };
    let uptime = host.uptime_ms();
    let paused = s.paused;
    let object_count = s.objects.len() as u32;
    let machine_count = agents.len() as u32;
    host.serve_info(|_| HostInfo {
        uptime_ms: uptime,
        pid: std::process::id(),
        paused: paused as u32,
        machine_count,
        object_count,
        props: Props::from_pairs(&[("name", "fake"), ("version", "fake")]).into_bytes(),
    });
    let mut paused = s.paused;
    host.serve_clock(|cmd| {
        match cmd.op {
            clock_op::PAUSE => paused = true,
            clock_op::PLAY => paused = false,
            clock_op::TOGGLE => paused = !paused,
            _ => {}
        }
        ClockState {
            step: 0,
            paused: paused as u32,
            _pad: 0,
        }
    });
    s.paused = paused;

    let mut objects = std::mem::take(&mut s.objects);
    let mut cleared = s.cleared;
    host.serve_clear(|_| {
        objects.clear();
        cleared += 1;
        Status::ok()
    });
    s.cleared = cleared;
    host.serve_list(|q| {
        objects
            .iter()
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
    let mut events = Vec::new();
    host.serve_usd_load(|req| {
        let id = req.id();
        objects.retain(|o| o.props().get("id").as_deref() != Some(id.as_str()));
        if req.remove() {
            events.push(SceneEvent::new(event_kind::REMOVED, &id));
            return Status::ok();
        }
        let kind = if req.is_machine() {
            object_kind::MACHINE
        } else {
            object_kind::PROP
        };
        objects.push(SceneObject {
            x: req.x,
            y: req.y,
            z: req.z,
            yaw_deg: req.yaw_deg,
            kind,
            props: req.props.clone(),
        });
        events.push(
            SceneEvent::new(event_kind::LOADED, &id)
                .at(req.x, req.y, req.z)
                .with_prop("path", &req.path().unwrap_or_default()),
        );
        if req.is_machine() {
            if let Some(ns) = req.namespace() {
                spawn.push(ns);
            }
        } else {
            // A real prop settles on the terrain first; here it lands at once.
            events.push(
                SceneEvent::new(event_kind::POSE, &id)
                    .at(req.x, req.y, req.z)
                    .with_top(req.y + 0.45),
            );
        }
        Status::ok()
    });
    host.serve_usd_delete(|req| {
        let id = req.id();
        objects.retain(|o| o.props().get("id").as_deref() != Some(id.as_str()));
        Status::ok()
    });
    s.objects = objects;
    for ev in &events {
        host.publish_event(ev);
    }

    let mut markers = std::mem::take(&mut s.markers);
    host.serve_marker_set(|req| {
        let id = req.id();
        markers.retain(|(m, _)| m != &id);
        markers.push((id, [req.x, req.y, req.z]));
        Status::ok()
    });
    host.serve_marker_delete(|req| {
        let id = req.id();
        markers.retain(|(m, _)| m != &id);
        Status::ok()
    });
    s.markers = markers;

    let mut selection = s.selection.take();
    host.serve_select(|req| {
        if req.query == 0 {
            selection = Some((req.kind, req.id()));
        }
        match &selection {
            Some((kind, id)) => Selection::set(*kind, id),
            None => Selection::default(),
        }
    });
    s.selection = selection;

    let refs: Vec<MachineRef> = agents.iter().map(|m| m.agent.machine_ref()).collect();
    host.serve_machines(|_| {
        refs.iter()
            .map(|r| MachineRef {
                props: r.props.clone(),
            })
            .collect()
    });
    for m in agents.iter_mut() {
        m.agent.poll();
    }
    spawn
}
