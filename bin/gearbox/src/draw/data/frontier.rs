use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::render::{extract_resource::ExtractResource, view::RetainedViewEntity};
use gearbox_api::draw_data::{Charge, CpuBudget, MemoryClass, StorageError};
use gearbox_api::wire::draw_data::ViewIdentity;

use super::worker::{Dataset, Scene, Snapshot, charge};
use super::{
    owned::List,
    retirement::{LiveList, Read},
};

#[cfg(test)]
#[path = "frontier_tests.rs"]
mod tests;

pub(super) struct DatasetSet {
    pub datasets: LiveList<Arc<Dataset>>,
    _charge: Charge,
}

impl DatasetSet {
    pub fn select(
        budget: &CpuBudget,
        previous: Option<&Arc<Self>>,
        candidates: &[Arc<Dataset>],
        live: impl Fn(&Dataset) -> bool,
        ready: impl Fn(&Dataset) -> bool,
    ) -> Result<Arc<Self>, StorageError> {
        let old = previous.map_or_else(Read::default, |set| set.datasets.read());
        let replacement = |data: &Dataset| {
            candidates.iter().find(|candidate| {
                candidate.key() == data.key()
                    && candidate.revision.incarnation == data.revision.incarnation
                    && candidate.revision.generation >= data.revision.generation
                    && live(candidate)
                    && ready(candidate)
            })
        };
        let retained = || {
            old.iter()
                .filter(|data| live(data))
                .map(|data| replacement(data).unwrap_or(data))
        };
        let added = || {
            candidates.iter().filter(|data| {
                live(data)
                    && ready(data)
                    && !old.iter().any(|old| live(old) && old.key() == data.key())
            })
        };
        let count = retained().count() + added().count();
        if let Some(previous) = previous
            && count == old.len()
            && count == old.capacity()
            && retained().chain(added()).all(|data| {
                old.binary_search_by_key(&(Arc::as_ptr(data) as usize), |data| {
                    Arc::as_ptr(data) as usize
                })
                .is_ok()
            })
        {
            return Ok(previous.clone());
        }
        let metadata = charge(budget, size_of::<Self>() + 16)?;
        let mut datasets = List::new(budget, count, MemoryClass::Metadata)?;
        for data in retained().chain(added()) {
            datasets.push(data.clone())?;
        }
        datasets.sort_unstable_by_key(|data| Arc::as_ptr(data) as usize);
        Ok(Arc::new(Self {
            datasets: datasets.into(),
            _charge: metadata,
        }))
    }
}

#[derive(Clone)]
pub(super) struct ViewFrontier {
    pub retained: RetainedViewEntity,
    pub identity: ViewIdentity,
    pub datasets: Arc<DatasetSet>,
    pub preparing: Option<Arc<DatasetSet>>,
}

pub(super) struct Selection {
    pub views: Vec<ViewFrontier>,
    _charge: Charge,
}

impl Selection {
    pub fn prune(&self) {
        for view in &self.views {
            view.datasets.datasets.prune();
            if let Some(preparing) = &view.preparing {
                preparing.datasets.prune();
            }
        }
    }
    pub fn new(budget: &CpuBudget, count: usize) -> Result<Self, StorageError> {
        let metadata = charge(
            budget,
            size_of::<Self>() + 16 + count * size_of::<ViewFrontier>(),
        )?;
        let mut views = Vec::new();
        views
            .try_reserve_exact(count)
            .map_err(|_| StorageError::Allocation)?;
        if views.capacity() != count {
            return Err(StorageError::Allocation);
        }
        Ok(Self {
            views,
            _charge: metadata,
        })
    }

    pub fn find(
        &self,
        retained: RetainedViewEntity,
        identity: ViewIdentity,
    ) -> Option<&Arc<DatasetSet>> {
        self.views
            .iter()
            .find(|view| view.retained == retained && view.identity == identity)
            .map(|view| &view.datasets)
    }
}

#[derive(Resource, Default)]
pub(super) struct Frontiers(pub Option<Arc<Selection>>);

#[derive(Default, Clone)]
pub(super) struct Holds {
    pub offered: Option<Arc<Scene>>,
    pub selected: Option<Arc<Selection>>,
    pub demand: Option<(u64, Arc<super::worker::Demand>)>,
}

pub(super) struct Retention {
    state: Mutex<Holds>,
    _charge: Charge,
}

impl Retention {
    pub fn new(budget: &CpuBudget) -> Result<Arc<Self>, StorageError> {
        Ok(Arc::new(Self {
            state: Mutex::new(Holds::default()),
            _charge: charge(budget, size_of::<Self>() + 16)?,
        }))
    }

    pub fn read(&self) -> Holds {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn offer(&self, scene: Option<Arc<Scene>>) {
        let old = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            std::mem::replace(&mut state.offered, scene)
        };
        drop(old);
    }

    /// Replaces the demanded page set when its stamp changes.
    pub fn demand(
        &self,
        stamp: u64,
        make: impl FnOnce() -> Result<super::worker::Demand, StorageError>,
    ) -> Result<(), StorageError> {
        if self
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .demand
            .as_ref()
            .is_some_and(|(old, _)| *old == stamp)
        {
            return Ok(());
        }
        let demand = Arc::new(make()?);
        let old = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            state.demand.replace((stamp, demand))
        };
        drop(old);
        Ok(())
    }

    pub fn select(&self, selection: Option<Arc<Selection>>) {
        let old = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            std::mem::replace(&mut state.selected, selection)
        };
        drop(old);
    }
}

#[derive(Resource, Clone)]
pub(super) struct Offered {
    pub snapshot: Option<Arc<Snapshot>>,
    pub scene: Option<Arc<Scene>>,
    pub retention: Arc<Retention>,
}

impl Offered {
    pub fn prune(&self) {
        if let Some(snapshot) = &self.snapshot {
            snapshot.entries.prune();
        }
        if let Some(scene) = &self.scene {
            scene.prune();
        }
        let held = self.retention.read();
        if let Some(scene) = held.offered {
            scene.prune();
        }
        if let Some(selection) = held.selected {
            selection.prune();
        }
    }
    pub fn live(&self, data: &Dataset) -> bool {
        data.lifetime.is_live()
            && self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.contains(data.key(), data.revision.incarnation))
    }

    pub fn candidates(&self) -> Read<'_, Arc<Dataset>> {
        self.scene
            .as_ref()
            .map_or_else(Read::default, |scene| scene.datasets.read())
    }

    pub fn visit_needed(&self, held: &Holds, mut visit: impl FnMut(&Arc<Dataset>)) {
        let mut live = |data: &Arc<Dataset>| {
            if self.live(data) {
                visit(data);
            }
        };
        for data in self.candidates().iter() {
            live(data);
        }
        if let Some(scene) = &held.offered {
            for data in scene.datasets.read().iter() {
                live(data);
            }
        }
        if let Some(selection) = &held.selected {
            for view in &selection.views {
                for data in view.datasets.datasets.read().iter() {
                    live(data);
                }
                if let Some(preparing) = &view.preparing {
                    for data in preparing.datasets.read().iter() {
                        live(data);
                    }
                }
            }
        }
    }
}

impl ExtractResource<bevy::render::RenderApp> for Offered {
    type Source = Self;

    fn extract_resource(source: &Self) -> Self {
        source.retention.offer(source.scene.clone());
        source.clone()
    }
}
