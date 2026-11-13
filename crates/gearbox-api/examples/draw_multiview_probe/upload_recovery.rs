use super::*;
use gearbox_api::WaitVisibleError;

fn retryable(error: &WaitVisibleError, expected: VisibilityTarget) -> bool {
    matches!(error,
        WaitVisibleError::Terminal(GenerationVisibility::Failed { target, failure })
            if *target == expected && failure.phase == DataPhase::Rendering
                && failure.code == data_code::BUDGET)
}

pub(super) fn wait(
    control: &Control,
    data: &mut DrawDataClient,
    key: &DataKey,
    revision: DatasetRevision,
    views: [Option<ViewIdentity>; 2],
    source_samples: u64,
) -> Result<serde_json::Value> {
    let started = Instant::now();
    let until = deadline();
    let mut receipts = Vec::with_capacity(2);
    for view in views {
        let view = view.ok_or("continuous recovery requires both original views")?;
        let target = VisibilityTarget {
            dataset: revision,
            view,
        };
        let mut failures = 0_u64;
        loop {
            match data.wait_visible_with_deadline(key, revision, Some(view), until) {
                Ok(receipt) => {
                    require(
                        receipt.target == target
                            && receipt.source_samples == source_samples
                            && receipt.selected_items == source_samples,
                        "continuous recovery returned an incomplete or different frontier",
                    )?;
                    receipts.push(serde_json::json!({"visibility":receipt.visibility(),
                        "budget_failures":failures,"elapsed_ms":started.elapsed().as_millis()}));
                    break;
                }
                Err(error) if retryable(&error, target) => {
                    failures += 1;
                    let report = control.read()?;
                    require(
                        number(&report.allocations, "gpu_bytes")
                            <= data.capabilities().limits.gpu_bytes
                            && number(&report.cpu, "committed")
                                <= data.capabilities().limits.cpu_bytes
                            && number(&report.allocations, "max_frame_upload")
                                <= data.capabilities().limits.upload_bytes_per_frame,
                        "continuous recovery exceeded a resource cap",
                    )?;
                    std::fs::write(
                        control.root.join("streaming-recovery-progress.json"),
                        serde_json::to_vec_pretty(&serde_json::json!({"target":target,
                            "failures":failures,"error":error.to_string(),"report":report}))?,
                    )?;
                    require(
                        Instant::now() < until,
                        "continuous recovery deadline exceeded",
                    )?;
                    std::thread::sleep(
                        Duration::from_millis(20)
                            .min(until.saturating_duration_since(Instant::now())),
                    );
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(
        serde_json::json!({"receipts":receipts,"elapsed_ms":started.elapsed().as_millis(),
        "deadline_ms":30_000}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policy_accepts_only_exact_rendering_budget_failures() {
        let target = VisibilityTarget {
            dataset: DatasetRevision {
                incarnation: 3,
                generation: 7,
            },
            view: ViewIdentity { id: 1, epoch: 2 },
        };
        let failure = DataFailure {
            phase: DataPhase::Rendering,
            code: data_code::BUDGET,
            reason: "GPU allocation denied".into(),
        };
        let error = |target, failure| {
            WaitVisibleError::Terminal(GenerationVisibility::Failed { target, failure })
        };
        assert!(retryable(&error(target, failure.clone()), target));
        assert!(!retryable(
            &error(
                target,
                DataFailure {
                    code: data_code::UNSUPPORTED,
                    ..failure.clone()
                }
            ),
            target
        ));
        assert!(!retryable(
            &error(
                target,
                DataFailure {
                    phase: DataPhase::Preparing,
                    ..failure.clone()
                }
            ),
            target
        ));
        let mut replaced = target;
        replaced.dataset.generation += 1;
        assert!(!retryable(&error(replaced, failure.clone()), target));
        replaced = target;
        replaced.view.epoch += 1;
        assert!(!retryable(&error(replaced, failure), target));
        assert!(!retryable(
            &WaitVisibleError::Terminal(GenerationVisibility::ViewGone { target }),
            target
        ));
    }
}
