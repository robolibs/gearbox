//! Retained debug draws served on `/gearbox/draw` and `/gearbox/draw/list`
//! by a thread of their own, into a store the viewer reads every frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bevy::prelude::*;

use super::{GearboxBus, SimResetRequest};
use crate::draw::touched_props;
use crate::draw_data::{DrawingBackend, LegacyDrawGuard, StreamingStore};
use crate::host::{DataLane, DrawLane};
use crate::wire::*;

/// The live draws, shared with the thread that serves the draw topics.
#[derive(Resource, Clone, Default)]
pub struct DrawStoreRes(Arc<DrawShared>);

#[derive(Default)]
struct DrawShared {
    store: DrawingBackend,
    /// Sim seconds lifetimes count from, as `f64` bits.
    now: AtomicU64,
}

impl DrawShared {
    fn lock(&self) -> LegacyDrawGuard<'_> {
        self.store.lock()
    }
}

impl DrawStoreRes {
    pub fn lock(&self) -> LegacyDrawGuard<'_> {
        self.0.lock()
    }
    pub fn backend(&self) -> DrawingBackend {
        self.0.store.clone()
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
            .add_systems(
                Update,
                (clear_draws_on_reset, tick_draws)
                    .chain()
                    .in_set(DrawSystems::Serve),
            );
    }
}

fn start_draw_server(
    mut commands: Commands,
    bus: Option<Res<GearboxBus>>,
    store: Res<DrawStoreRes>,
) {
    if let Err(error) = crate::draw_data::survey::register_host_providers(&store.0.store) {
        warn!("draw data page providers: {error}");
    }
    let Some(bus) = bus else {
        return;
    };
    let lane = bus.host.draw_lane().clone();
    let data_lane = bus.host.draw_data_lane().clone();
    let shared = store.0.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("gearbox-draw".into())
        .spawn(move || serve_loop(&lane, &data_lane, &shared, &flag))
        .map_err(|err| error!("gearbox-api: draw server thread failed to start: {err}"))
        .ok();
    commands.insert_resource(DrawServer { stop, thread });
}

/// Polls every 0.1 ms while draws arrive, every 2 ms once they stop.
fn serve_loop(lane: &DrawLane, data_lane: &DataLane, shared: &DrawShared, stop: &AtomicBool) {
    const BUSY: Duration = Duration::from_micros(100);
    const IDLE: Duration = Duration::from_millis(2);
    const QUIET: Duration = Duration::from_millis(500);
    let mut last = Instant::now();
    let started = Instant::now();
    let mut storage = match StreamingStore::with_backend(data_lane.epoch(), shared.store.clone()) {
        Ok(storage) => storage,
        Err(error) => {
            error!("gearbox-api: drawing backend: {error}");
            return;
        }
    };
    while !stop.load(Ordering::Relaxed) {
        let now = f64::from_bits(shared.now.load(Ordering::Relaxed));
        if let Err(error) = storage.advance_simulation(now) {
            error!("gearbox-api: spatial clock: {error}");
        }
        let monotonic = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let data_served = data_lane
            .serve_store(&mut storage, monotonic)
            .unwrap_or_else(|error| {
                error!("gearbox-api: spatial data service: {error}");
                0
            });
        let served = data_served
            + lane.serve(
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
    if store.pending_len() > 0 || store.iter().any(|(_, item)| item.expires_s.is_some()) {
        store.expire(now);
    }
}

fn clear_draws_on_reset(mut messages: MessageReader<SimResetRequest>, store: Res<DrawStoreRes>) {
    if messages
        .read()
        .any(|m| matches!(m.scope, clear_scope::ALL | clear_scope::MARKERS))
    {
        store.lock().clear("");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(now: f64) -> App {
        let mut app = App::new();
        app.init_resource::<Time>()
            .add_message::<SimResetRequest>()
            .insert_resource(DrawClock(Some(now)))
            .add_plugins(DrawServicePlugin);
        app.update();
        app
    }

    fn tick(app: &mut App, now: f64) {
        app.world_mut().resource_mut::<DrawClock>().0 = Some(now);
        app.update();
    }

    fn chunks(ns: &str) -> [DrawEntity; 2] {
        let mut first = DrawEntity::points(ns, "scan", &[[1.0, 2.0, 3.0]]);
        first.op = draw_op::APPEND;
        first.seq = 7;
        first.last = 0;
        let mut last = first.clone();
        last.chunk_offset = 1;
        last.last = 1;
        [first, last]
    }

    #[test]
    fn pending_upload_expires_without_live_draws() {
        let mut app = app(10.0);
        let [first, last] = chunks("expiry/empty");
        let shared = app.world().resource::<DrawStoreRes>().clone();
        shared.lock().apply(first, 10.0).unwrap();
        assert_eq!(shared.lock().pending_len(), 1);
        tick(&mut app, 40.0);
        let mut store = shared.lock();
        assert_eq!(store.pending_len(), 0);
        assert!(store.is_empty());
        assert!(store.apply(last, 40.0).is_err());
    }

    #[test]
    fn pending_replacement_expires_with_only_permanent_draws() {
        let mut app = app(10.0);
        let shared = app.world().resource::<DrawStoreRes>().clone();
        let original = DrawEntity::points("expiry/permanent", "scan", &[[9.0; 3]]);
        let [first, last] = chunks("expiry/permanent");
        shared.lock().apply(original.clone(), 10.0).unwrap();
        let generation = shared
            .lock()
            .get("expiry/permanent", "scan")
            .unwrap()
            .generation;
        shared.lock().apply(first, 10.0).unwrap();
        tick(&mut app, 40.0);
        let mut store = shared.lock();
        assert_eq!(store.pending_len(), 0);
        assert!(store.apply(last, 40.0).is_err());
        let retained = store.get("expiry/permanent", "scan").unwrap();
        assert_eq!(retained.entity.points, original.points);
        assert_eq!(retained.generation, generation);
    }

    #[test]
    fn pending_timeout_uses_simulation_time_not_update_count() {
        let mut app = app(10.0);
        let shared = app.world().resource::<DrawStoreRes>().clone();
        let [first, last] = chunks("expiry/paused");
        shared.lock().apply(first, 10.0).unwrap();
        for _ in 0..100 {
            app.update();
        }
        assert_eq!(shared.lock().pending_len(), 1);
        tick(&mut app, 39.999);
        assert_eq!(shared.lock().pending_len(), 1);
        shared.lock().apply(last, 39.999).unwrap();
        tick(&mut app, 40.0);
        let store = shared.lock();
        assert_eq!(store.pending_len(), 0);
        assert_eq!(
            store
                .get("expiry/paused", "scan")
                .unwrap()
                .entity
                .point_count(),
            2
        );
    }

    #[test]
    fn simulation_rewind_discards_pending_uploads() {
        let mut app = app(20.0);
        let shared = app.world().resource::<DrawStoreRes>().clone();
        let [first, last] = chunks("expiry/rewind");
        shared.lock().apply(first, 20.0).unwrap();
        shared
            .lock()
            .apply(DrawEntity::points("kept", "point", &[[0.0; 3]]), 20.0)
            .unwrap();
        tick(&mut app, 19.0);
        let mut store = shared.lock();
        assert_eq!(store.pending_len(), 0);
        assert!(store.get("kept", "point").is_some());
        assert!(store.apply(last, 19.0).is_err());
    }

    #[test]
    fn scene_reset_cancels_pending_uploads_before_tick() {
        for scope in [clear_scope::ALL, clear_scope::MARKERS] {
            let mut app = app(10.0);
            let shared = app.world().resource::<DrawStoreRes>().clone();
            let [first, last] = chunks("expiry/reset");
            shared.lock().apply(first, 10.0).unwrap();
            app.world_mut()
                .write_message(SimResetRequest { scope, ..default() });
            app.update();
            let mut store = shared.lock();
            assert_eq!(store.pending_len(), 0);
            assert!(store.apply(last, 10.0).is_err());
        }
    }

    #[test]
    fn scene_reset_shares_dataset_ownership_and_revokes_pending_publication() {
        use crate::draw_data::{DatasetMetadata, DatasetStorage, Generation};
        use crate::wire::draw_data::*;
        for scope in [
            clear_scope::ALL,
            clear_scope::MARKERS,
            clear_scope::MACHINES,
            clear_scope::PROPS,
        ] {
            let mut app = app(10.0);
            let shared = app.world().resource::<DrawStoreRes>().clone();
            let backend = shared.backend();
            let mut storage = StreamingStore::with_backend(17, backend.clone()).unwrap();
            let descriptor = DataDescriptor {
                revision: SCHEMA_REVISION,
                key: DataKey::new("reset", "occupancy").unwrap(),
                shape: DataShape::Octree {
                    config: OctreeConfig {
                        leaf_resolution_m: 0.05,
                        max_depth: 12,
                        free_max: 0.3,
                        occupied_min: 0.7,
                    },
                    frame: DataFrame::default(),
                },
                stream: StreamMode::RetainedMap,
                lifetime_s: None,
                presentation: Presentation::for_octree(),
            };
            let metadata = DatasetMetadata::new(backend.budget(), &descriptor).unwrap();
            let generation = Generation::build(backend.budget(), metadata, &[], &[]).unwrap();
            let weak = Arc::downgrade(&generation);
            let registry = backend.registry();
            let permit = {
                let mut registry = registry.lock().unwrap();
                let first = registry
                    .begin_dataset(&descriptor.key, DatasetKind::Octree, None)
                    .unwrap();
                let storage = DatasetStorage::from_resident(backend.budget(), generation).unwrap();
                let revision = registry.commit_dataset(first, storage).unwrap();
                registry
                    .begin_dataset(&descriptor.key, DatasetKind::Octree, Some(revision))
                    .unwrap()
            };
            assert!(
                shared
                    .lock()
                    .apply(DrawEntity::points("reset", "occupancy", &[[0.0; 3]]), 10.0)
                    .is_err()
            );
            app.world_mut()
                .write_message(SimResetRequest { scope, ..default() });
            app.update();
            let clears_data = matches!(scope, clear_scope::ALL | clear_scope::MARKERS);
            {
                let registry = registry.lock().unwrap();
                assert_eq!(registry.dataset_len(), usize::from(!clears_data));
                assert_eq!(registry.permit_is_live(permit), !clears_data);
            }
            assert!(weak.upgrade().is_some());
            storage.maintain(1).unwrap();
            assert_eq!(weak.upgrade().is_none(), clears_data);
        }
    }

    #[test]
    fn namespace_clear_keeps_neighbor_upload_and_rejects_late_chunk() {
        let mut app = app(10.0);
        let shared = app.world().resource::<DrawStoreRes>().clone();
        let [first, last] = chunks("expiry/one");
        let [neighbor, end] = chunks("expiry/onetwo");
        {
            let mut store = shared.lock();
            store.apply(first, 10.0).unwrap();
            store.apply(neighbor, 10.0).unwrap();
            store.clear("expiry/one");
            assert_eq!(store.pending_len(), 1);
            assert!(store.apply(last, 10.0).is_err());
            store.apply(end, 10.0).unwrap();
        }
        tick(&mut app, 40.0);
        assert!(shared.lock().get("expiry/onetwo", "scan").is_some());
    }

    #[test]
    fn wrong_sequence_drops_pending_but_preserves_the_live_generation() {
        let mut app = app(10.0);
        let shared = app.world().resource::<DrawStoreRes>().clone();
        let original = DrawEntity::points("expiry/sequence", "scan", &[[9.0; 3]]);
        let [first, mut last] = chunks("expiry/sequence");
        {
            let mut store = shared.lock();
            store.apply(original.clone(), 10.0).unwrap();
            let generation = store.get("expiry/sequence", "scan").unwrap().generation;
            store.apply(first, 10.0).unwrap();
            last.seq += 1;
            assert!(store.apply(last, 10.0).is_err());
            assert_eq!(store.pending_len(), 0);
            let retained = store.get("expiry/sequence", "scan").unwrap();
            assert_eq!(retained.entity.points, original.points);
            assert_eq!(retained.generation, generation);
        }
        tick(&mut app, 40.0);
        assert_eq!(shared.lock().len(), 1);
    }
}
