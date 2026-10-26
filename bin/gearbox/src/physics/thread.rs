//! Physics on its own thread. After `PostUpdate`, the frame's last physics
//! reader, the world and the control tick's state cross to the thread, which
//! steps them in real time while the frame extracts, renders, presents and
//! takes input; before the next `Update` they come back. The frame waits on
//! no more than the step in progress, so a step slower than real time slows
//! the simulation, not the frame, and a slow frame does not slow the
//! simulation: between steps the thread answers the API's drive commands and
//! clock reads and publishes the machines' states. A step budget runs at most
//! `MAX_STEPS_PER_FRAME` steps a frame, as the frame's own stepping does.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bevy::app::MainScheduleOrder;
use bevy::ecs::component::Mutable;
use bevy::ecs::schedule::{ScheduleLabel, SingleThreadedExecutor};
use bevy::prelude::*;

use super::PhysicsActive;
use super::world::{ControlTick, MAX_STEPS_PER_FRAME, PhysicsWorld, StepStats, tick};

/// Most real time physics may fall behind; past it the simulation slows down
/// instead of catching up in a burst.
const MAX_DEBT_S: f64 = 0.25;

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct PhysicsJoin;

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct PhysicsLaunch;

/// The resources the control tick runs on, and how each crosses.
#[derive(Resource, Default)]
pub(crate) struct TickResources {
    launch: Vec<fn(&mut World, &mut World)>,
    join: Vec<fn(&mut World, &mut World)>,
}

/// Values the control tick and the frame both write while the thread runs.
pub(crate) trait Merge {
    /// Take what the tick changed from `sent` into `back`; leave the rest as the frame has it.
    fn merge(&mut self, sent: &Self, back: Self);
}

pub(crate) trait TickResourcesExt {
    /// The control tick reads `R`: the frame's copy crosses at each launch.
    fn tick_reads<R: Resource + Clone>(&mut self) -> &mut Self;
    /// The control tick writes `R`: a copy crosses at each launch and comes
    /// back at each join, the frame reading its own in between.
    fn tick_writes<R: Resource + Clone>(&mut self) -> &mut Self;
    /// As `tick_writes`, but what the frame changed in between survives
    /// wherever the tick left it alone.
    fn tick_merges<R: Resource<Mutability = Mutable> + Clone + Merge>(&mut self) -> &mut Self;
}

impl TickResourcesExt for App {
    fn tick_reads<R: Resource + Clone>(&mut self) -> &mut Self {
        self.world_mut().get_resource_or_init::<TickResources>().launch.push(copy::<R>);
        self
    }

    fn tick_writes<R: Resource + Clone>(&mut self) -> &mut Self {
        let mut resources = self.world_mut().get_resource_or_init::<TickResources>();
        resources.launch.push(copy::<R>);
        resources.join.push(take::<R>);
        self
    }

    fn tick_merges<R: Resource<Mutability = Mutable> + Clone + Merge>(&mut self) -> &mut Self {
        let mut resources = self.world_mut().get_resource_or_init::<TickResources>();
        resources.launch.push(send_merged::<R>);
        resources.join.push(merge_back::<R>);
        self
    }
}

fn copy<R: Resource + Clone>(from: &mut World, to: &mut World) {
    if let Some(value) = from.get_resource::<R>() {
        to.insert_resource(value.clone());
    }
}

fn take<R: Resource>(from: &mut World, to: &mut World) {
    if let Some(value) = from.remove_resource::<R>() {
        to.insert_resource(value);
    }
}

/// What a merged resource was when it crossed.
#[derive(Resource)]
struct Sent<R>(R);

fn send_merged<R: Resource + Clone>(frame: &mut World, tick: &mut World) {
    if let Some(value) = frame.get_resource::<R>().cloned() {
        tick.insert_resource(value.clone());
        frame.insert_resource(Sent(value));
    }
}

fn merge_back<R: Resource<Mutability = Mutable> + Merge>(tick: &mut World, frame: &mut World) {
    let (Some(back), Some(Sent(sent))) = (tick.remove_resource::<R>(), frame.remove_resource::<Sent<R>>()) else {
        return;
    };
    match frame.get_resource_mut::<R>() {
        Some(mut own) => own.merge(&sent, back),
        None => frame.insert_resource(back),
    }
}

/// Moves `ControlTick` and the stepping onto a thread of their own.
pub struct PhysicsThreadPlugin;

impl Plugin for PhysicsThreadPlugin {
    fn build(&self, app: &mut App) {
        app.init_schedule(PhysicsJoin)
            .init_schedule(PhysicsLaunch)
            .add_systems(PhysicsJoin, join)
            .add_systems(PhysicsLaunch, launch)
            .tick_reads::<Time>()
            .tick_reads::<PhysicsActive>()
            .tick_reads::<crate::prim_index::PrimIndex>();
        let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
        order.insert_after(PreUpdate, PhysicsJoin);
        order.insert_after(PostUpdate, PhysicsLaunch);
    }

    fn cleanup(&self, app: &mut App) {
        let world = app.world_mut();
        let mut schedule = world
            .resource_mut::<Schedules>()
            .remove(ControlTick)
            .unwrap_or_else(|| Schedule::new(ControlTick));
        schedule.set_executor(SingleThreadedExecutor::new());
        let mut tick = World::new();
        tick.add_schedule(schedule);
        tick.insert_resource(OnPhysicsThread);
        world.insert_resource(PhysicsThread::spawn(tick));
    }
}

/// Marks the thread's world: what runs there runs between steps.
#[derive(Resource)]
pub(crate) struct OnPhysicsThread;

pub(crate) fn on_the_thread(marker: Option<Res<OnPhysicsThread>>) -> bool {
    marker.is_some()
}

/// The physics stays with the frame: no thread, or a paused clock kept it.
pub(crate) fn physics_on_the_frame(thread: Option<Res<PhysicsThread>>) -> bool {
    thread.is_none_or(|thread| thread.in_flight.is_none())
}

/// How many steps a batch runs.
#[derive(Clone, Copy)]
enum Plan {
    /// Exactly this many, before the frame takes the world back: a step budget.
    Exact(u32),
    /// As many as real time owes since `origin`; the frame takes the world
    /// back after the step in progress.
    RealTime { origin: Instant, hz: f64 },
}

impl Plan {
    fn due(&self, steps: u32) -> bool {
        match *self {
            Plan::Exact(n) => steps < n,
            Plan::RealTime { origin, hz } => (steps + 1) as f64 <= origin.elapsed().as_secs_f64() * hz,
        }
    }
}

struct Batch {
    world: World,
    plan: Plan,
}

struct Ran {
    world: World,
    steps: u32,
    seconds: f64,
}

/// The physics thread and, while the frame holds the physics, its world.
#[derive(Resource)]
pub(crate) struct PhysicsThread {
    batches: SyncSender<Batch>,
    ran: Mutex<Receiver<Ran>>,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
    idle: Option<World>,
    in_flight: Option<Plan>,
    launched: Instant,
    /// Where real time stands against the steps run: the last step's due time.
    origin: Instant,
    stats: StepStats,
    waits: Waits,
}

impl PhysicsThread {
    fn spawn(world: World) -> Self {
        let (batches, inbox) = sync_channel::<Batch>(1);
        let (outbox, ran) = sync_channel::<Ran>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::Builder::new()
            .name("gearbox-physics".into())
            .spawn(move || run(inbox, outbox, stopped))
            .expect("physics thread");
        Self {
            batches,
            ran: Mutex::new(ran),
            stop,
            thread,
            idle: Some(world),
            in_flight: None,
            launched: Instant::now(),
            origin: Instant::now(),
            stats: StepStats::default(),
            waits: Waits::default(),
        }
    }
}

impl Drop for PhysicsThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.thread().unpark();
    }
}

/// How long the thread held the physics and how long the frame then waited
/// on the step in progress, logged every 10 s.
#[derive(Default)]
struct Waits {
    away: f64,
    waited: f64,
    joins: u32,
    since: Option<Instant>,
}

impl Waits {
    fn record(&mut self, away: f64, waited: f64) {
        self.away += away;
        self.waited += waited;
        self.joins += 1;
        let since = *self.since.get_or_insert_with(Instant::now);
        let span = since.elapsed().as_secs_f64();
        if span >= 10.0 {
            info!(
                "gearbox-physics: the thread held the world {:.0}% of the time, {:.1} ms a frame; the frame waited {:.2} ms a frame on it",
                100.0 * self.away / span,
                1000.0 * self.away / self.joins as f64,
                1000.0 * self.waited / self.joins as f64
            );
            *self = Self { since: Some(Instant::now()), ..default() };
        }
    }
}

fn run(inbox: Receiver<Batch>, outbox: SyncSender<Ran>, stop: Arc<AtomicBool>) {
    while let Ok(Batch { mut world, plan }) = inbox.recv() {
        let (mut steps, mut seconds) = (0u32, 0.0);
        loop {
            let stopping = stop.load(Ordering::Acquire);
            if plan.due(steps) && !(stopping && matches!(plan, Plan::RealTime { .. })) {
                let started = Instant::now();
                tick(&mut world);
                seconds += started.elapsed().as_secs_f64();
                steps += 1;
                continue;
            }
            if stopping || matches!(plan, Plan::Exact(_)) {
                break;
            }
            if let Plan::RealTime { hz, .. } = plan {
                std::thread::park_timeout(Duration::from_secs_f64(0.5 / hz));
            }
        }
        if outbox.send(Ran { world, steps, seconds }).is_err() {
            return;
        }
    }
}

/// Take the physics back from the thread before the frame's first reader.
fn join(world: &mut World) {
    let Some(mut thread) = world.remove_resource::<PhysicsThread>() else {
        return;
    };
    if let Some(plan) = thread.in_flight.take() {
        let waiting = Instant::now();
        thread.stop.store(true, Ordering::Release);
        thread.thread.thread().unpark();
        let Ran { world: mut tick, steps, seconds } =
            thread.ran.lock().expect("physics thread inbox").recv().expect("physics thread stopped");
        let away = thread.launched.elapsed().as_secs_f64();
        thread.waits.record(away, waiting.elapsed().as_secs_f64());
        take::<PhysicsWorld>(&mut tick, world);
        let resources = world.remove_resource::<TickResources>().unwrap_or_default();
        for back in &resources.join {
            back(&mut tick, world);
        }
        world.insert_resource(resources);
        thread.idle = Some(tick);
        let mut physics = world.resource_mut::<PhysicsWorld>();
        physics.pending_steps = steps;
        if let Plan::RealTime { origin, hz } = plan {
            let behind = (origin.elapsed().as_secs_f64() - steps as f64 / hz).clamp(0.0, MAX_DEBT_S);
            thread.origin = Instant::now() - Duration::from_secs_f64(behind);
            physics.accumulator = behind.min(1.0 / hz);
        }
        if std::mem::take(&mut physics.pause_after_steps) {
            world.resource_mut::<PhysicsActive>().0 = false;
        }
        thread.stats.record(seconds, steps, world.resource::<PhysicsWorld>());
    }
    world.insert_resource(thread);
}

/// Hand the physics to the thread after the frame's last reader. A paused
/// clock keeps it on the frame.
fn launch(world: &mut World) {
    let Some(mut thread) = world.remove_resource::<PhysicsThread>() else {
        return;
    };
    let active = world.get_resource::<PhysicsActive>().is_some_and(|a| a.0);
    if !active || !world.contains_resource::<PhysicsWorld>() {
        thread.origin = Instant::now();
        world.insert_resource(thread);
        return;
    }
    let budget = world.get_resource_mut::<gearbox_api::PhysicsStepBudget>().and_then(|mut budget| {
        let left = budget.0?;
        let steps = left.min(MAX_STEPS_PER_FRAME);
        budget.0 = (left > steps).then_some(left - steps);
        Some((steps, budget.0.is_none()))
    });
    let plan = match budget {
        Some((steps, last)) => {
            world.resource_mut::<PhysicsWorld>().pause_after_steps = last;
            thread.origin = Instant::now();
            Plan::Exact(steps)
        }
        None => Plan::RealTime { origin: thread.origin, hz: world.resource::<PhysicsWorld>().step_hz },
    };
    let mut tick = thread.idle.take().expect("physics thread world");
    let resources = world.remove_resource::<TickResources>().unwrap_or_default();
    for send in &resources.launch {
        send(world, &mut tick);
    }
    world.insert_resource(resources);
    take::<PhysicsWorld>(world, &mut tick);
    thread.stop.store(false, Ordering::Release);
    thread.batches.send(Batch { world: tick, plan }).expect("physics thread stopped");
    thread.in_flight = Some(plan);
    thread.launched = Instant::now();
    world.insert_resource(thread);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::backend::{BodyDesc, BodyId, ColliderDesc, Pose, Shape};
    use glam::DVec3;

    #[derive(Resource, Clone, Default)]
    struct Command(f64);

    #[derive(Resource, Clone, Default)]
    struct Seen {
        runs: u64,
        command: f64,
    }

    fn follow(command: Res<Command>, mut seen: ResMut<Seen>) {
        seen.runs += 1;
        seen.command = command.0;
    }

    /// A ball dropped from 10 m, its physics on the thread, and a control
    /// tick that records the command it saw.
    fn app() -> (App, BodyId) {
        let mut physics = PhysicsWorld::default();
        let ball = physics.insert_body(BodyDesc::dynamic().pose(Pose::from_translation(DVec3::Y * 10.0)));
        physics.insert_collider(ColliderDesc::new(Shape::Ball { radius: 0.1 }).density(1000.0).parent(ball)).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, PhysicsThreadPlugin))
            .init_schedule(ControlTick)
            .add_systems(ControlTick, follow)
            .insert_resource(physics)
            .insert_resource(PhysicsActive(true))
            .init_resource::<crate::prim_index::PrimIndex>()
            .init_resource::<Command>()
            .init_resource::<Seen>()
            .tick_reads::<Command>()
            .tick_writes::<Seen>();
        app.finish();
        app.cleanup();
        (app, ball)
    }

    /// Run `frames` frames 5 ms apart, then take the physics back.
    fn run(app: &mut App, frames: u32) {
        for _ in 0..frames {
            app.update();
            std::thread::sleep(Duration::from_millis(5));
        }
        app.world_mut().run_schedule(PhysicsJoin);
    }

    #[test]
    fn the_thread_steps_in_real_time_and_the_frame_reads_its_results() {
        let (mut app, ball) = app();
        let started = Instant::now();
        run(&mut app, 40);
        let elapsed = started.elapsed().as_secs_f64();
        let physics = app.world().resource::<PhysicsWorld>();
        let height = physics.body(ball).unwrap().position().translation.y;
        assert!(height < 9.9, "the ball fell to {height}");
        let simulated = physics.simulated_seconds;
        assert!(simulated > 0.5 * elapsed && simulated <= elapsed + 1.0 / physics.step_hz, "{simulated} s simulated in {elapsed} s");
    }

    #[test]
    fn a_control_tick_sees_the_frame_s_command_and_its_writes_come_back() {
        let (mut app, _) = app();
        app.world_mut().resource_mut::<Command>().0 = 3.0;
        run(&mut app, 20);
        let ticks = app.world().resource::<PhysicsWorld>().ticks;
        let seen = app.world().resource::<Seen>();
        assert!(ticks > 0);
        assert_eq!(seen.runs, ticks);
        assert_eq!(seen.command, 3.0);
    }

    #[test]
    fn a_slow_frame_does_not_hold_the_physics_back() {
        let (mut app, _) = app();
        for _ in 0..2 {
            app.update();
            std::thread::sleep(Duration::from_millis(400));
            app.world_mut().run_schedule(PhysicsJoin);
        }
        let physics = app.world().resource::<PhysicsWorld>();
        let owed = 0.8 * physics.step_hz;
        assert!(physics.ticks as f64 >= 0.8 * owed && physics.ticks as f64 <= owed + 1.0, "{} ticks", physics.ticks);
    }

    #[test]
    fn a_step_budget_runs_exactly_its_ticks_and_pauses() {
        let (mut app, _) = app();
        app.insert_resource(gearbox_api::PhysicsStepBudget(Some(5)));
        run(&mut app, 4);
        assert_eq!(app.world().resource::<PhysicsWorld>().ticks, 5);
        assert!(!app.world().resource::<PhysicsActive>().0);
    }

    #[test]
    fn a_paused_clock_keeps_the_physics_on_the_frame_and_its_budget() {
        let (mut app, _) = app();
        app.insert_resource(PhysicsActive(false)).insert_resource(gearbox_api::PhysicsStepBudget(Some(5)));
        run(&mut app, 3);
        assert!(app.world().contains_resource::<PhysicsWorld>());
        assert_eq!(app.world().resource::<PhysicsWorld>().ticks, 0);
        assert_eq!(app.world().resource::<gearbox_api::PhysicsStepBudget>().0, Some(5));
    }
}
