use std::sync::Mutex;

use super::{
    Charge, CpuBudget, DatasetStorage, DrawingBackend, DrawingStore, MemoryClass, StorageError,
};
use crate::wire::draw_data::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderCompletion {
    pub target: VisibilityTarget,
    pub frame: u64,
    pub disposition: FrontierDisposition,
    pub source_samples: u64,
    pub selected_items: u64,
}

impl RenderCompletion {
    pub fn visibility(self) -> GenerationVisibility {
        GenerationVisibility::Visible {
            target: self.target,
            frame: self.frame,
            disposition: self.disposition,
            source_samples: self.source_samples,
            selected_items: self.selected_items,
        }
    }
}

struct Slot {
    key: DataKey,
    view: ViewIdentity,
    incarnation: u64,
    last: RenderCompletion,
    expired_through: u64,
}

struct History {
    key: DataKey,
    completion: RenderCompletion,
}

pub(super) struct Book {
    views: Vec<ViewIdentity>,
    primary: Option<ViewIdentity>,
    kinds: Vec<DatasetKind>,
    slots: Vec<Slot>,
    history: Vec<History>,
    max_views: usize,
    max_slots: usize,
    max_history: usize,
    registry_revision: u64,
    _charge: Charge,
}

pub(super) type Visibility = Mutex<Option<Book>>;

fn reserved<T>(count: usize) -> Result<Vec<T>, StorageError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| StorageError::Allocation)?;
    if values.capacity() != count {
        return Err(StorageError::Allocation);
    }
    Ok(values)
}

impl Book {
    fn new(
        budget: &CpuBudget,
        limits: &DataLimits,
        max_views: usize,
        kinds: &[DatasetKind],
    ) -> Result<Self, StorageError> {
        if !(1..=64).contains(&max_views)
            || kinds.is_empty()
            || kinds.len() > 16
            || kinds
                .iter()
                .enumerate()
                .any(|(i, kind)| kinds[..i].contains(kind))
        {
            return Err(StorageError::Invalid(
                "invalid renderer visibility configuration".into(),
            ));
        }
        let max_slots = limits.datasets as usize * max_views;
        let max_history = limits.completion_history as usize;
        let bytes = max_views * size_of::<ViewIdentity>()
            + kinds.len() * size_of::<DatasetKind>()
            + max_slots * (size_of::<Slot>() + 382)
            + max_history * (size_of::<History>() + 382);
        let charge = budget
            .reserve(bytes as u64)?
            .charge(bytes as u64, MemoryClass::Metadata)?;
        let mut supported = reserved(kinds.len())?;
        supported.extend_from_slice(kinds);
        Ok(Self {
            views: reserved(max_views)?,
            primary: None,
            kinds: supported,
            slots: reserved(max_slots)?,
            history: reserved(max_history)?,
            max_views,
            max_slots,
            max_history,
            registry_revision: u64::MAX,
            _charge: charge,
        })
    }

    fn slot(&self, key: &DataKey, target: VisibilityTarget) -> Option<usize> {
        self.slots.iter().position(|slot| {
            slot.key == *key
                && slot.view == target.view
                && slot.incarnation == target.dataset.incarnation
        })
    }

    fn prune(&mut self, registry: &DrawingStore<DatasetStorage>) {
        if self.registry_revision == registry.revision() {
            return;
        }
        self.slots.retain(|slot| {
            registry
                .get(&(slot.key.namespace.clone(), slot.key.id.clone()))
                .is_some_and(|entry| entry.revision.incarnation == slot.incarnation)
        });
        self.history.retain(|record| {
            registry
                .get(&(record.key.namespace.clone(), record.key.id.clone()))
                .is_some_and(|entry| {
                    entry.revision.incarnation == record.completion.target.dataset.incarnation
                })
        });
        self.registry_revision = registry.revision();
    }

    fn archive(&mut self, history: History) {
        if self.history.len() == self.max_history {
            let removed = self.history.remove(0);
            if let Some(index) = self.slot(&removed.key, removed.completion.target) {
                self.slots[index].expired_through = self.slots[index]
                    .expired_through
                    .max(removed.completion.target.dataset.generation);
            }
        }
        self.history.push(history);
    }
}

impl DrawingBackend {
    pub fn enable_renderer_visibility(
        &self,
        max_views: usize,
        kinds: &[DatasetKind],
    ) -> Result<(), StorageError> {
        let mut state = self.0.visibility.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(book) = &*state {
            if book.max_views != max_views || book.kinds != kinds {
                return Err(StorageError::Invalid(
                    "renderer visibility already configured differently".into(),
                ));
            }
        } else {
            *state = Some(Book::new(self.budget(), self.limits(), max_views, kinds)?);
        }
        Ok(())
    }

    pub fn renderer_kinds(&self) -> Vec<DatasetKind> {
        self.0
            .visibility
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map_or_else(Vec::new, |book| book.kinds.clone())
    }

    pub fn sync_render_views(
        &self,
        views: &[ViewIdentity],
        primary: Option<ViewIdentity>,
    ) -> Result<(), StorageError> {
        for view in views {
            view.validate().map_err(StorageError::Invalid)?;
        }
        if primary.is_some_and(|primary| !views.contains(&primary))
            || views
                .iter()
                .enumerate()
                .any(|(i, view)| views[..i].iter().any(|old| old.id == view.id))
        {
            return Err(StorageError::Invalid(
                "invalid primary or duplicate renderer view".into(),
            ));
        }
        let mut state = self.0.visibility.lock().unwrap_or_else(|p| p.into_inner());
        let book = state
            .as_mut()
            .ok_or_else(|| StorageError::Invalid("renderer visibility disabled".into()))?;
        if views.len() > book.max_views {
            return Err(StorageError::Allocation);
        }
        book.views.clear();
        book.views.extend_from_slice(views);
        book.primary = primary;
        book.slots.retain(|slot| views.contains(&slot.view));
        book.history
            .retain(|record| views.contains(&record.completion.target.view));
        Ok(())
    }

    pub fn complete_render_frame(
        &self,
        key: &DataKey,
        completion: RenderCompletion,
    ) -> Result<bool, StorageError> {
        key.validate().map_err(StorageError::Invalid)?;
        completion
            .visibility()
            .validate()
            .map_err(StorageError::Invalid)?;
        let registry = self.0.registry.lock().unwrap_or_else(|p| p.into_inner());
        let Some(entry) = registry
            .get(&(key.namespace.clone(), key.id.clone()))
            .filter(|entry| entry.revision == completion.target.dataset)
        else {
            return Ok(false);
        };
        let counts = entry.payload.counts();
        if completion.source_samples != counts.source_samples
            || completion.selected_items > counts.valid_samples
            || (completion.disposition == FrontierDisposition::Empty && counts.valid_samples != 0)
            || (completion.disposition == FrontierDisposition::Culled && counts.valid_samples == 0)
        {
            return Err(StorageError::Invalid(
                "completion counts do not match retained data".into(),
            ));
        }
        let mut state = self.0.visibility.lock().unwrap_or_else(|p| p.into_inner());
        let Some(book) = state.as_mut() else {
            return Ok(false);
        };
        if !book.kinds.contains(&entry.kind) || !book.views.contains(&completion.target.view) {
            return Ok(false);
        }
        book.prune(&registry);
        if let Some(index) = book.slot(key, completion.target) {
            let last = book.slots[index].last;
            if completion.frame <= last.frame {
                return Ok(false);
            }
            if last.target.dataset != completion.target.dataset {
                book.archive(History {
                    key: key.clone(),
                    completion: last,
                });
            }
            book.slots[index].last = completion;
        } else {
            if book.slots.len() >= book.max_slots {
                return Err(StorageError::Allocation);
            }
            book.slots.push(Slot {
                key: key.clone(),
                view: completion.target.view,
                incarnation: completion.target.dataset.incarnation,
                last: completion,
                expired_through: 0,
            });
        }
        Ok(true)
    }

    pub fn render_visibility(
        &self,
        key: &DataKey,
        requested: DatasetRevision,
        view: Option<ViewIdentity>,
    ) -> GenerationVisibility {
        let registry = self.0.registry.lock().unwrap_or_else(|p| p.into_inner());
        self.visibility_for(&registry, key, requested, view)
    }

    pub(super) fn visibility_for(
        &self,
        registry: &DrawingStore<DatasetStorage>,
        key: &DataKey,
        requested: DatasetRevision,
        view: Option<ViewIdentity>,
    ) -> GenerationVisibility {
        let unavailable = |reason: &str| GenerationVisibility::Unavailable {
            reason: reason.into(),
        };
        let Some(entry) = registry
            .get(&(key.namespace.clone(), key.id.clone()))
            .filter(|entry| {
                entry.revision.incarnation == requested.incarnation
                    && entry.revision.generation >= requested.generation
            })
        else {
            return unavailable("requested dataset incarnation is no longer retained");
        };
        let mut state = self.0.visibility.lock().unwrap_or_else(|p| p.into_inner());
        let Some(book) = state.as_mut() else {
            return unavailable("dataset renderer is not available");
        };
        if !book.kinds.contains(&entry.kind) {
            return unavailable("dataset kind has no renderer");
        }
        book.prune(registry);
        let Some(view) = view.or(book.primary) else {
            return unavailable("no eligible drawing view");
        };
        let target = VisibilityTarget {
            dataset: requested,
            view,
        };
        if !book.views.contains(&view) {
            return GenerationVisibility::ViewGone { target };
        }
        if let Some(index) = book.slot(key, target) {
            let slot = &book.slots[index];
            if slot.last.target.dataset == requested {
                return slot.last.visibility();
            }
            if let Some(history) = book
                .history
                .iter()
                .find(|record| record.key == *key && record.completion.target == target)
            {
                return history.completion.visibility();
            }
            if requested.generation <= slot.expired_through {
                return GenerationVisibility::HistoryExpired { target };
            }
        }
        if requested.generation < entry.revision.generation {
            return GenerationVisibility::Superseded {
                target,
                successor: entry.revision,
            };
        }
        drop(state);
        if let Some(failure) = self.render_failure(key, requested) {
            return GenerationVisibility::Failed { target, failure };
        }
        GenerationVisibility::Pending { target }
    }
}
