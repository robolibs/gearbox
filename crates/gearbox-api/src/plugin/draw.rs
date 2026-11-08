//! Retained debug draws served on `/gearbox/draw` and `/gearbox/draw/list`
//! by a thread of their own, into a store the viewer reads every frame.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bevy::prelude::*;

use super::{GearboxBus, SimResetRequest};
use crate::draw::{DrawStore, touched_props};
use crate::host::DrawLane;
use crate::wire::*;

/// The live draws, shared with the thread that serves the draw topics.
#[derive(Resource, Clone, Default)]
pub struct DrawStoreRes(Arc<DrawShared>);

#[derive(Default)]
struct DrawShared {
    store: Mutex<DrawStore>,
    /// Sim seconds lifetimes count from, as `f64` bits.
    now: AtomicU64,
}

impl DrawShared {
    fn lock(&self) -> MutexGuard<'_, DrawStore> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl DrawStoreRes {
    pub fn lock(&self) -> MutexGuard<'_, DrawStore> {
        self.0.lock()
    }
}

/// Sim seconds draw lifetimes count in; the simulator sets it every frame,
/// else the app's own clock counts.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct DrawClock(pub Option<f64>);

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum DrawSystems {
    /// Where the simulator sets [`DrawClock`].
    Clock,
    /// Lifetimes run out and scene clears land.
    Serve,
}

/// The serving thread, stopped and joined when the app drops it.
#[derive(Resource)]
struct DrawServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for DrawServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct DrawServicePlugin;

impl Plugin for DrawServicePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DrawStoreRes>()
            .init_resource::<DrawClock>()
            .configure_sets(Update, (DrawSystems::Clock, DrawSystems::Serve).chain())
            .add_systems(Startup, start_draw_server)
            .add_systems(Update, (clear_draws_on_reset, tick_draws).chain().in_set(DrawSystems::Serve));
    }
}

fn start_draw_server(mut commands: Commands, bus: Option<Res<GearboxBus>>, store: Res<DrawStoreRes>) {
    let Some(bus) = bus else {
        return;
    };
    let lane = bus.host.draw_lane().clone();
    let shared = store.0.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("gearbox-draw".into())
        .spawn(move || serve_loop(&lane, &shared, &flag))
        .map_err(|err| error!("gearbox-api: draw server thread failed to start: {err}"))
        .ok();
    commands.insert_resource(DrawServer { stop, thread });
}

/// Polls every 0.1 ms while draws arrive, every 2 ms once they stop.
fn serve_loop(lane: &DrawLane, shared: &DrawShared, stop: &AtomicBool) {
    const BUSY: Duration = Duration::from_micros(100);
    const IDLE: Duration = Duration::from_millis(2);
    const QUIET: Duration = Duration::from_millis(500);
    let mut last = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let now = f64::from_bits(shared.now.load(Ordering::Relaxed));
        let served = lane.serve(
            |msg| match shared.lock().apply(msg, now) {
                Ok(touched) => Status::ok_with(&touched_props(touched)),
                Err(err) => Status::err(code::USAGE, &err),
            },
            |query| shared.lock().summaries(&query.ns()),
        );
        if served > 0 {
            last = Instant::now();
        }
        std::thread::sleep(if last.elapsed() < QUIET { BUSY } else { IDLE });
    }
}

fn tick_draws(store: Res<DrawStoreRes>, clock: Res<DrawClock>, time: Res<Time>) {
    let now = clock.0.unwrap_or_else(|| time.elapsed_secs_f64());
    store.0.now.store(now.to_bits(), Ordering::Relaxed);
    let mut store = store.lock();
    if store.iter().any(|(_, item)| item.expires_s.is_some()) {
        store.expire(now);
    }
}

fn clear_draws_on_reset(mut messages: MessageReader<SimResetRequest>, store: Res<DrawStoreRes>) {
    if messages.read().any(|m| matches!(m.scope, clear_scope::ALL | clear_scope::MARKERS)) {
        store.lock().clear("");
    }
}
