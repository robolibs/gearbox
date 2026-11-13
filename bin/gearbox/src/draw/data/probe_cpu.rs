use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

use gearbox_api::draw_data::{CpuBudget, Reservation};
use serde::Serialize;

#[derive(Default)]
struct State {
    headroom: Option<u64>,
    hold: Option<Reservation>,
}

#[derive(Default)]
pub(in crate::draw::data) struct Pressure {
    state: Mutex<State>,
    failures: AtomicU64,
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct Report {
    pub held: u64,
    pub allocated: u64,
    pub page_bytes: u64,
    pub metadata_bytes: u64,
    pub scratch_bytes: u64,
    pub committed: u64,
    pub limit: u64,
    pub snapshot_failures: u64,
}

impl Pressure {
    pub fn configure(&self, headroom: Option<u64>) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.headroom = headroom;
        if headroom.is_none() {
            state.hold = None;
        }
    }

    pub fn apply(&self, budget: &CpuBudget) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let Some(target) = state.headroom else { return };
        let usage = budget.usage();
        let available = usage.limit - usage.committed();
        if available > target {
            if let Ok(extra) = budget.reserve(available - target) {
                if let Some(hold) = &mut state.hold {
                    hold.merge(extra).expect("same probe CPU ledger");
                } else {
                    state.hold = Some(extra);
                }
            }
        } else if let Some(hold) = &mut state.hold {
            let release = hold.bytes().min(target - available);
            drop(hold.split(release).expect("bounded pressure release"));
        }
    }

    pub fn failed_snapshot(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn report(&self, budget: &CpuBudget) -> Report {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let usage = budget.usage();
        Report {
            held: state.hold.as_ref().map_or(0, Reservation::bytes),
            allocated: usage.allocated,
            page_bytes: usage.page_bytes,
            metadata_bytes: usage.metadata_bytes,
            scratch_bytes: usage.scratch_bytes,
            committed: usage.committed(),
            limit: usage.limit,
            snapshot_failures: self.failures.load(Ordering::Relaxed),
        }
    }
}

#[test]
fn pressure_tracks_released_headroom_without_changing_production_limits() {
    let budget = CpuBudget::new(1024);
    let owner = budget.reserve(256).unwrap();
    let pressure = Pressure::default();
    pressure.configure(Some(0));
    pressure.apply(&budget);
    assert!(budget.reserve(1).is_err());
    assert_eq!(pressure.report(&budget).held, 768);
    drop(owner);
    pressure.apply(&budget);
    assert_eq!(pressure.report(&budget).held, 1024);
    pressure.configure(Some(32));
    pressure.apply(&budget);
    assert_eq!(budget.usage().committed(), 992);
    pressure.failed_snapshot();
    assert_eq!(pressure.report(&budget).snapshot_failures, 1);
    pressure.configure(None);
    assert_eq!(budget.usage().committed(), 0);
    assert_eq!(budget.usage().limit, 1024);
}
