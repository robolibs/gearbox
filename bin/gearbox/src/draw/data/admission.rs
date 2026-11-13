use gearbox_api::draw_data::{Charge, CpuBudget, DrawingBackend, RenderDiagnostic, StorageError};
use gearbox_api::wire::draw_data::{DataPhase, data_code};

use super::worker::{Dataset, charge};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Issue {
    Deferred,
    CpuBudget,
    GpuAllocation,
    GpuStaging,
    DeviceLimit,
    InvalidSelection,
}

impl Issue {
    pub fn failure(self) -> Option<RenderDiagnostic> {
        let (code, reason) = match self {
            Self::InvalidSelection => (
                data_code::INVALID,
                "invalid cloud selection indices or point budget; previous cut retained",
            ),
            Self::Deferred => return None,
            Self::CpuBudget => (
                data_code::BUDGET,
                "cloud CPU staging/metadata budget exhausted; preparation will retry",
            ),
            Self::GpuAllocation => (
                data_code::BUDGET,
                "cloud GPU allocation budget exhausted; preparation will retry",
            ),
            Self::GpuStaging => (
                data_code::BUDGET,
                "cloud GPU staging budget exhausted; preparation will retry",
            ),
            Self::DeviceLimit => (
                data_code::UNSUPPORTED,
                "cloud buffer exceeds GPU device buffer or binding limit",
            ),
        };
        Some(
            RenderDiagnostic::new(DataPhase::Rendering, code, reason)
                .expect("static rendering diagnostic"),
        )
    }

    pub fn report(self, backend: &DrawingBackend, data: &Dataset) {
        if let Some(failure) = self.failure()
            && backend.renderer_diagnostic(data.key(), data.revision) != Some(failure)
            && let Err(error) =
                backend.report_renderer_diagnostic(data.key(), data.revision, failure)
        {
            tracing::warn!("cloud admission status: {error}");
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Outcome {
    pages: usize,
    issue: Option<Issue>,
}

impl Outcome {
    fn record(&mut self, result: Result<(), Issue>) {
        self.pages += 1;
        self.issue = self.issue.max(result.err());
    }

    fn complete(&self, expected: usize) -> bool {
        self.pages == expected && self.issue.is_none()
    }
}

pub(super) struct Results<'a> {
    pages: Vec<(&'a Dataset, Result<(), Issue>)>,
    _charge: Charge,
}

impl<'a> Results<'a> {
    pub fn new(budget: &CpuBudget, count: usize) -> Result<Self, StorageError> {
        let metadata = charge(budget, count * size_of::<(&Dataset, Result<(), Issue>)>())?;
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(count)
            .map_err(|_| StorageError::Allocation)?;
        if pages.capacity() != count {
            return Err(StorageError::Allocation);
        }
        Ok(Self {
            pages,
            _charge: metadata,
        })
    }

    pub fn record(&mut self, data: &'a Dataset, result: Result<(), Issue>) {
        self.pages.push((data, result));
    }

    pub fn publish(mut self, backend: &DrawingBackend) {
        self.pages
            .sort_unstable_by_key(|(data, _)| *data as *const Dataset as usize);
        for group in self.pages.chunk_by(|a, b| std::ptr::eq(a.0, b.0)) {
            let data = group[0].0;
            let mut outcome = Outcome::default();
            for (_, result) in group {
                outcome.record(*result);
            }
            if let Some(issue) = outcome.issue {
                issue.report(backend, data);
            } else if outcome.complete(data.render_pages())
                && !backend
                    .renderer_diagnostic(data.key(), data.revision)
                    .is_some_and(|error| error.reason().starts_with("drawing frost "))
            {
                backend.clear_render_failure(data.key(), data.revision, DataPhase::Rendering);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_ready_pages_do_not_erase_failure() {
        for results in [
            [Err(Issue::GpuAllocation), Ok(())],
            [Ok(()), Err(Issue::GpuAllocation)],
        ] {
            let mut outcome = Outcome::default();
            for result in results {
                outcome.record(result);
            }
            assert_eq!(outcome.issue, Some(Issue::GpuAllocation));
            assert!(!outcome.complete(2));
        }
    }

    #[test]
    fn deferred_uploads_are_not_failures_or_complete() {
        let mut outcome = Outcome::default();
        outcome.record(Err(Issue::Deferred));
        outcome.record(Ok(()));
        assert!(!outcome.complete(2));
        assert!(outcome.issue.unwrap().failure().is_none());
    }

    #[test]
    fn complete_requires_every_page() {
        let mut outcome = Outcome::default();
        outcome.record(Ok(()));
        assert!(!outcome.complete(2));
        outcome.record(Ok(()));
        assert!(outcome.complete(2));
    }

    #[test]
    fn failures_are_valid_exact_rendering_reports() {
        for issue in [
            Issue::CpuBudget,
            Issue::GpuAllocation,
            Issue::GpuStaging,
            Issue::DeviceLimit,
        ] {
            let failure = issue.failure().unwrap();
            assert_eq!(failure.phase(), DataPhase::Rendering);
            assert_ne!(failure.code(), data_code::OK);
            assert!(!failure.reason().is_empty());
        }
    }

    #[test]
    fn hard_failure_wins_over_frame_throttle() {
        let mut outcome = Outcome::default();
        outcome.record(Err(Issue::GpuAllocation));
        outcome.record(Err(Issue::Deferred));
        assert_eq!(outcome.issue, Some(Issue::GpuAllocation));
    }

    #[test]
    fn preparation_results_are_charged_and_released() {
        let budget = CpuBudget::new(1024);
        let before = budget.usage().committed();
        let results = Results::new(&budget, 8).unwrap();
        assert!(budget.usage().committed() > before);
        assert!(Results::new(&budget, 1024).is_err());
        drop(results);
        assert_eq!(budget.usage().committed(), before);
    }
}
