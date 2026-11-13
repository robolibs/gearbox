use bevy::prelude::*;
use gearbox_api::draw_data::{CpuBudget, MemoryClass, StorageError};

use super::admission::Issue;
use super::owned::{List, Map};

pub(super) const QUANTUM: u64 = 64 * 1024;

pub(super) fn install(app: &mut App) {
    use bevy::render::{Render, RenderApp};
    app.sub_app_mut(RenderApp)
        .init_resource::<Scheduler>()
        .init_resource::<super::upload_window::Window>()
        .add_systems(
            Render,
            dispatch.in_set(super::render::Preparation::Dispatch),
        );
}

fn dispatch(
    mut typed: super::render::Prepare,
    mut legacy: super::legacy::render::Prepare,
    mut octree: super::octree::render::Prepare,
    mut scheduler: ResMut<Scheduler>,
    mut upload: ResMut<super::render::UploadFrame>,
    mut window: ResMut<super::upload_window::Window>,
    shared: Option<Res<super::runtime::Shared>>,
    queue: Res<bevy::render::renderer::RenderQueue>,
) {
    typed.prune();
    legacy.prune();
    octree.prune();
    let Some(shared) = shared else { return };
    #[cfg(feature = "draw-probes")]
    if let Some(probe) = &shared.probe {
        probe.begin_upload_frame();
    }
    let budget = shared.backend.budget();
    scheduler.prune(
        budget,
        typed
            .jobs()
            .chain(legacy.jobs())
            .chain(octree.jobs())
            .map(|job| job.key),
    );
    if typed.count() == 0 && legacy.count() == 0 && octree.count() == 0 {
        window.clear();
        return;
    }
    let permit = match window.acquire(budget) {
        Ok(permit) => permit,
        Err(_) => {
            typed.deny();
            return;
        }
    };
    let allowance = if permit.is_some() {
        upload.remaining
    } else {
        0
    };
    let prepare = || {
        let count = typed
            .count()
            .checked_add(legacy.count())
            .and_then(|count| count.checked_add(octree.count()))
            .ok_or(StorageError::Allocation)?;
        let mut jobs = List::new(budget, count, MemoryClass::Scratch)?;
        for job in typed.jobs().chain(legacy.jobs()).chain(octree.jobs()) {
            jobs.push(job)?;
        }
        let slots = staging_slots(typed.count(), legacy.count() + octree.count(), allowance)?;
        let staging = List::new(budget, slots, MemoryClass::Scratch)?;
        let results = super::admission::Results::new(budget, typed.count())?;
        Ok::<_, StorageError>((jobs, staging, results))
    };
    let Ok((mut jobs, mut staging, results)) = prepare() else {
        typed.deny();
        return;
    };
    let mut remaining = allowance;
    let result = scheduler.run(budget, &mut jobs, &mut remaining, |key, item, credit| {
        #[cfg(feature = "draw-probes")]
        let before = *credit;
        let result = match key {
            Key::Typed(_) => typed.step(item, credit, &mut staging),
            Key::Legacy(_) => legacy.step(item, credit, &mut staging),
            Key::Octree(_) => octree.step(item, credit, &mut staging),
            Key::Selection(..) => typed.step_selection(key, item, credit, &mut staging),
        };
        #[cfg(feature = "draw-probes")]
        if let Some(probe) = &shared.probe {
            probe.group_uploaded(key, before - *credit);
        }
        result
    });
    upload.remaining -= allowance - remaining;
    if result.is_ok() {
        typed.publish(&jobs, results);
        legacy.publish(&jobs);
        octree.publish(&jobs);
    } else {
        typed.deny();
    }
    if !staging.is_empty() {
        queue.submit([]);
        queue.on_submitted_work_done(move || {
            drop(staging);
            drop(permit);
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Key {
    Typed(u64),
    Legacy(u64),
    Octree(u64),
    Selection(u64, u64),
}

pub(super) struct Job {
    pub key: Key,
    pub item: u64,
    pub result: Result<(), Issue>,
    active: bool,
}

impl Job {
    pub fn new(key: Key, item: u64) -> Self {
        Self {
            key,
            item,
            result: Err(Issue::Deferred),
            active: true,
        }
    }
}

struct Group {
    key: Key,
    start: usize,
    end: usize,
}

#[derive(Resource, Default)]
pub(super) struct Scheduler {
    cursors: Map<Key, u64>,
    next: Option<Key>,
}

impl Scheduler {
    pub fn prune(&mut self, budget: &CpuBudget, keys: impl IntoIterator<Item = Key>) {
        self.cursors.retain_keys(keys, |_, _| {});
        let _ = self.cursors.compact(budget);
        if self.cursors.len() == 0 {
            self.next = None;
        } else if let Some(next) = self.next
            && !self.cursors.contains_key(&next)
        {
            self.next = self
                .cursors
                .iter()
                .map(|(key, _)| *key)
                .filter(|key| *key >= next)
                .min()
                .or_else(|| self.cursors.iter().map(|(key, _)| *key).min());
        }
    }

    pub fn run(
        &mut self,
        budget: &CpuBudget,
        jobs: &mut [Job],
        remaining: &mut u64,
        mut step: impl FnMut(Key, u64, &mut u64) -> Result<(), Issue>,
    ) -> Result<(), StorageError> {
        jobs.sort_unstable_by_key(|job| (job.key, job.item));
        let count = jobs.chunk_by(|a, b| a.key == b.key).count();
        let mut groups = List::new(budget, count, MemoryClass::Scratch)?;
        let mut start = 0;
        for group in jobs.chunk_by(|a, b| a.key == b.key) {
            let key = group[0].key;
            if !self.cursors.contains_key(&key) {
                self.cursors.insert(budget, key, group[0].item)?;
            }
            let end = start + group.len();
            groups.push(Group { key, start, end })?;
            start = end;
        }
        if groups.is_empty() {
            return Ok(());
        }
        let mut index = self
            .next
            .map_or(0, |next| groups.partition_point(|group| group.key < next))
            % groups.len();
        let mut idle = 0;
        while *remaining >= 4 && idle < groups.len() {
            let group = &groups[index];
            let items = &mut jobs[group.start..group.end];
            let cursor = self.cursors.get_mut(&group.key).unwrap();
            let mut item = items.partition_point(|job| job.item < *cursor) % items.len();
            let credit = (*remaining).min(QUANTUM) / 4 * 4;
            let mut allowance = credit;
            for _ in 0..items.len() {
                let job = &mut items[item];
                if job.active {
                    let before = allowance;
                    job.result = step(job.key, job.item, &mut allowance);
                    assert!(allowance <= before, "upload turn increased its byte credit");
                    if job.result == Err(Issue::Deferred) {
                        *cursor = job.item;
                        break;
                    }
                    job.active = false;
                }
                item = (item + 1) % items.len();
                *cursor = items[item].item;
                if allowance == 0 {
                    break;
                }
            }
            let used = credit - allowance;
            *remaining -= used;
            if used > 0 {
                self.next = Some(groups[(index + 1) % groups.len()].key);
                idle = 0;
            } else {
                idle += 1;
            }
            index = (index + 1) % groups.len();
        }
        for job in jobs.iter_mut().filter(|job| job.active) {
            let mut zero = 0;
            job.result = step(job.key, job.item, &mut zero);
            assert_eq!(zero, 0, "zero-credit readiness pass changed its allowance");
        }
        Ok(())
    }
}

pub(super) fn staging_slots(typed: usize, legacy: usize, cap: u64) -> Result<usize, StorageError> {
    if typed == 0 && legacy == 0 {
        return Ok(0);
    }
    typed
        .checked_mul(4)
        .and_then(|n| legacy.checked_mul(3).and_then(|m| n.checked_add(m)))
        .and_then(|n| {
            usize::try_from(cap / QUANTUM + u64::from(cap % QUANTUM != 0))
                .ok()
                .and_then(|extra| n.checked_add(extra))
        })
        .ok_or(StorageError::Allocation)
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;
