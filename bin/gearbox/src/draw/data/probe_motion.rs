use gearbox_api::wire::draw_data::DataLimits;
use serde::Serialize;

use super::{Allocations, LodReport};

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct ViewReport {
    frames: u64,
    missing: u64,
    empty: u64,
    invalid: u64,
    preparing: u64,
    coarse: u64,
    camera_changes: u64,
    selection_changes: u64,
    completions: u64,
    invalid_completions: u64,
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct Audit {
    active: bool,
    sequence: u64,
    start_frame: u64,
    end_frame: u64,
    frames: u64,
    failed_frames: u64,
    skipped_frames: u64,
    expected_sources: [u64; 2],
    views: [ViewReport; 2],
    max_gpu_bytes: u64,
    max_cpu_bytes: u64,
    max_frame_upload: u64,
    #[serde(skip)]
    cameras: [Option<u64>; 2],
    #[serde(skip)]
    selections: [Option<u64>; 2],
    #[serde(skip)]
    completed: [u64; 2],
}

impl Audit {
    pub fn begin(&mut self, sequence: u64, frame: u64, sources: [u64; 2]) {
        *self = Self {
            active: true,
            sequence,
            start_frame: frame,
            end_frame: frame,
            expected_sources: sources,
            ..Default::default()
        };
    }

    pub fn end(&mut self) {
        self.active = false;
    }

    pub fn observe(
        &mut self,
        frame: u64,
        limits: &DataLimits,
        cpu_bytes: u64,
        allocations: Allocations,
        cuts: [Option<LodReport>; 2],
        cameras: [Option<u64>; 2],
    ) {
        if !self.active || frame <= self.end_frame {
            return;
        }
        let skipped = frame - self.end_frame - 1;
        self.skipped_frames += skipped;
        self.end_frame = frame;
        self.frames += 1;
        self.max_gpu_bytes = self.max_gpu_bytes.max(allocations.gpu_bytes);
        self.max_cpu_bytes = self.max_cpu_bytes.max(cpu_bytes);
        self.max_frame_upload = self.max_frame_upload.max(allocations.frame_upload);
        let mut failed = skipped > 0
            || allocations.gpu_bytes > limits.gpu_bytes
            || cpu_bytes > limits.cpu_bytes
            || allocations.frame_upload > limits.upload_bytes_per_frame;
        for index in 0..2 {
            let view = &mut self.views[index];
            view.frames += 1;
            view.camera_changes +=
                u64::from(self.cameras[index].is_some() && self.cameras[index] != cameras[index]);
            self.cameras[index] = cameras[index];
            let Some(cut) = cuts[index] else {
                view.missing += 1;
                failed = true;
                continue;
            };
            let invalid = cut.valid != self.expected_sources[index]
                || cut.represented.checked_add(cut.culled) != Some(cut.valid)
                || cut.selected > u64::from(limits.visible_points)
                || cut.selected > cut.represented
                || !cut.verified_ordinals
                || cameras[index].is_none();
            view.empty += u64::from(cut.selected == 0);
            view.invalid += u64::from(invalid);
            view.preparing += u64::from(cut.preparing);
            view.coarse += u64::from(cut.coarse_pages > 0);
            view.selection_changes += u64::from(
                self.selections[index].is_some()
                    && self.selections[index] != Some(cut.selection_view),
            );
            self.selections[index] = Some(cut.selection_view);
            failed |= invalid || cut.selected == 0;
        }
        self.failed_frames += u64::from(failed);
    }

    pub fn completed(&mut self, index: usize, frame: u64, sources: u64, selected: u64) {
        if !self.active || frame <= self.start_frame || frame <= self.completed[index] {
            return;
        }
        self.completed[index] = frame;
        let view = &mut self.views[index];
        view.completions += 1;
        view.invalid_completions += u64::from(
            sources != self.expected_sources[index] || selected == 0 || selected > sources,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(stamp: u64) -> LodReport {
        LodReport {
            valid: 100,
            represented: 100,
            selected: 50,
            verified_ordinals: true,
            selection_view: stamp,
            ..Default::default()
        }
    }

    #[test]
    fn audit_retains_a_single_empty_frame_between_valid_observations() {
        let mut audit = Audit::default();
        audit.begin(7, 10, [100; 2]);
        for frame in 11..21 {
            let mut cuts = [Some(cut(frame)); 2];
            if frame == 14 {
                cuts[1].as_mut().unwrap().selected = 0;
            }
            audit.observe(
                frame,
                &DataLimits::default(),
                0,
                Allocations::default(),
                cuts,
                [Some(frame); 2],
            );
            audit.completed(0, frame, 100, 50);
        }
        assert_eq!(audit.frames, 10);
        assert_eq!(audit.failed_frames, 1);
        assert_eq!(audit.views[1].empty, 1);
        assert_eq!(audit.views[0].camera_changes, 9);
        assert_eq!(audit.views[0].selection_changes, 9);
        assert_eq!(audit.views[0].completions, 10);
        audit.end();
        audit.observe(
            21,
            &DataLimits::default(),
            0,
            Allocations::default(),
            [None; 2],
            [None; 2],
        );
        audit.completed(0, 21, 100, 0);
        assert_eq!(audit.frames, 10);
        assert_eq!(audit.views[0].completions, 10);
    }

    #[test]
    fn audit_rejects_unobserved_frames_without_counting_duplicate_observations() {
        let mut audit = Audit::default();
        audit.begin(1, 10, [100; 2]);
        for frame in [11, 11, 14] {
            audit.observe(
                frame,
                &DataLimits::default(),
                0,
                Allocations::default(),
                [Some(cut(frame)); 2],
                [Some(frame); 2],
            );
        }
        assert_eq!(audit.frames, 2);
        assert_eq!(audit.skipped_frames, 2);
        assert_eq!(audit.failed_frames, 1);
    }

    #[test]
    fn audit_rejects_missing_invalid_and_over_budget_frames_and_stale_receipts() {
        let limits = DataLimits::default();
        let mut audit = Audit::default();
        audit.begin(1, 20, [100; 2]);
        let mut invalid = cut(1);
        invalid.culled = 1;
        audit.observe(
            21,
            &limits,
            limits.cpu_bytes + 1,
            Allocations {
                gpu_bytes: limits.gpu_bytes + 1,
                frame_upload: limits.upload_bytes_per_frame + 1,
                ..Default::default()
            },
            [Some(invalid), None],
            [Some(1); 2],
        );
        assert_eq!(audit.failed_frames, 1);
        assert_eq!(audit.views[0].invalid, 1);
        assert_eq!(audit.views[1].missing, 1);
        audit.completed(0, 19, 100, 0);
        audit.completed(0, 21, 100, 0);
        audit.completed(0, 21, 100, 0);
        assert_eq!(audit.views[0].completions, 1);
        assert_eq!(audit.views[0].invalid_completions, 1);
        audit.begin(2, 30, [200; 2]);
        assert_eq!(audit.frames, 0);
        assert_eq!(audit.failed_frames, 0);
        assert_eq!(audit.views[0].completions, 0);
    }
}
