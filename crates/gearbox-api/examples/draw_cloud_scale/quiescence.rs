use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Cpu {
    held: u64,
    allocated: u64,
    committed: u64,
    limit: u64,
    page_bytes: u64,
    metadata_bytes: u64,
    scratch_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Receipts {
    frame: u64,
    paused: bool,
    in_flight: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct Baseline {
    pub resources: DataResourceUsage,
    cpu: Cpu,
    receipts: Receipts,
}

struct Pause<'a> {
    root: &'a Path,
    sequence: u64,
    active: bool,
}

fn control(root: &Path, paused: bool) -> Result<u64> {
    let sequence = ["report.json", "control.json"]
        .into_iter()
        .filter_map(|name| std::fs::read(root.join(name)).ok())
        .filter_map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter_map(|value| value["sequence"].as_u64())
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("probe control sequence exhausted")?;
    let bytes = serde_json::to_vec(&serde_json::json!({
        "sequence":sequence,"action":"remove","active":false,
        "max_generation":0,"pause_receipts":paused,
    }))?;
    let temporary = root.join("control.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, root.join("control.json"))?;
    Ok(sequence)
}

impl<'a> Pause<'a> {
    fn new(root: &'a Path) -> Result<Self> {
        Ok(Self {
            root,
            sequence: control(root, true)?,
            active: true,
        })
    }

    fn release(&mut self) -> Result<u64> {
        let sequence = control(self.root, false)?;
        self.active = false;
        Ok(sequence)
    }
}

impl Drop for Pause<'_> {
    fn drop(&mut self) {
        if self.active
            && let Err(error) = self.release()
        {
            eprintln!("could not resume receipt admission: {error}");
        }
    }
}

fn report(root: &Path) -> Option<Report> {
    serde_json::from_slice(&std::fs::read(root.join("report.json")).ok()?).ok()
}

fn quiescent(report: &Report, sequence: u64, resources: &DataResourceUsage) -> bool {
    report.sequence == sequence
        && report.receipts.frame > 0
        && report.receipts.paused
        && report.receipts.in_flight == 0
        && report.allocations.empty()
        && resources.gpu_bytes == 0
        && report.cpu.held == 0
        && report.cpu.allocated == resources.cpu_bytes
        && report.cpu.committed == resources.cpu_bytes
        && report.cpu.metadata_bytes == resources.metadata_bytes
        && report.cpu.committed <= report.cpu.limit
}

pub(super) fn baseline(data: &mut DrawDataClient, key: &DataKey, root: &Path) -> Result<Baseline> {
    let mut pause = Pause::new(root)?;
    let until = deadline();
    let mut previous = None;
    let mut stable = 0;
    let baseline = loop {
        let resources = status(data, key)?.resources;
        if let Some(report) = report(root)
            && quiescent(&report, pause.sequence, &resources)
        {
            stable = if previous.as_ref() == Some(&resources) {
                stable + 1
            } else {
                0
            };
            if stable >= 3 {
                break Baseline {
                    resources,
                    cpu: report.cpu,
                    receipts: report.receipts,
                };
            }
            previous = Some(resources);
        } else {
            stable = 0;
            previous = None;
        }
        if Instant::now() >= until {
            return Err(format!("zero-receipt baseline did not settle: {previous:?}").into());
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    let sequence = pause.release()?;
    let until = deadline();
    loop {
        if let Some(report) = report(root)
            && report.sequence == sequence
            && !report.receipts.paused
            && report.receipts.frame > baseline.receipts.frame
        {
            return Ok(baseline);
        }
        if Instant::now() >= until {
            return Err("receipt admission did not resume".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_requires_drained_receipts_and_unmodified_public_accounting() {
        let resources = DataResourceUsage {
            cpu_bytes: 100,
            metadata_bytes: 100,
            ..Default::default()
        };
        let mut report = Report {
            uploads: Default::default(),
            sequence: 3,
            allocations: Allocations::default(),
            cpu: Cpu {
                held: 0,
                allocated: 100,
                committed: 100,
                limit: 1000,
                page_bytes: 0,
                metadata_bytes: 100,
                scratch_bytes: 0,
            },
            receipts: Receipts {
                frame: 8,
                paused: true,
                in_flight: 0,
            },
        };
        assert!(quiescent(&report, 3, &resources));
        assert!(!quiescent(&report, 2, &resources));
        report.receipts.in_flight = 1;
        assert!(!quiescent(&report, 3, &resources));
        report.receipts.in_flight = 0;
        report.receipts.paused = false;
        assert!(!quiescent(&report, 3, &resources));
        report.receipts.paused = true;
        report.cpu.held = 1;
        assert!(!quiescent(&report, 3, &resources));
        report.cpu.held = 0;
        report.cpu.allocated = 99;
        assert!(!quiescent(&report, 3, &resources));
    }

    #[test]
    fn failed_measurement_resumes_with_a_new_sequence_before_report_ack() {
        let root =
            std::env::temp_dir().join(format!("gearbox-receipt-pause-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("report.json"), br#"{"sequence":40}"#).unwrap();
        let failure = (|| -> Result<()> {
            let pause = Pause::new(&root)?;
            assert_eq!(pause.sequence, 41);
            let request: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join("control.json"))?)?;
            assert_eq!(request["pause_receipts"], true);
            assert_eq!(request["action"], "remove");
            Err("deliberate measurement failure".into())
        })();
        assert!(failure.is_err());
        let request: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("control.json")).unwrap()).unwrap();
        assert_eq!(request["pause_receipts"], false);
        assert_eq!(request["sequence"], 42);
        std::fs::remove_dir_all(root).unwrap();
    }
}
