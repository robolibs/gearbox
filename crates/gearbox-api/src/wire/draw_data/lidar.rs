//! Native `gearbox.lidar_scan.v1` chunks assembled into organized typed cloud scans.
//!
//! Chunks group by machine, link, sample and capture time. Every pulse keeps its
//! organized index as its source ID; misses (infinite or NaN ranges) and pulses
//! that never arrived are explicit invalid observations, never zero points.
//! Placement is a rigid sweep pose: no per-pulse deskew is implied.

use super::{
    Attribute, AttributeSemantic, CloudPage, CloudSchema, ColumnData, DataFrame,
    MAX_PAGE_POINTS, NativeLidarCapture, Normalization, Organization, Placement, RigidPose,
    SCHEMA_REVISION, ScalarType, ScanDescriptor,
};
use crate::wire::LidarScan;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LidarLimits {
    /// Pulses in one sweep (`rows * cols`).
    pub max_pulses: u64,
    /// Incomplete sweeps kept open at once; the oldest is abandoned beyond this.
    pub max_open: usize,
}

impl Default for LidarLimits {
    fn default() -> Self {
        Self {
            max_pulses: 1 << 20,
            max_open: 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    /// Only sweeps with every pulse delivered are returned.
    CompleteOnly,
    /// Abandoned sweeps are returned too, with their missing pulses invalid.
    Partial,
}

/// Fields every chunk of one sweep must repeat exactly.
#[derive(Clone, Debug, PartialEq)]
struct Header {
    sim_time_s: f64,
    angles: [f64; 4],
    max_range_m: f64,
    link_index: u32,
    rows: u32,
    cols: u32,
    sample: u32,
    stamp_ms: u32,
}

impl Header {
    fn of(chunk: &LidarScan) -> Self {
        Self {
            sim_time_s: chunk.sim_time_s,
            angles: [chunk.theta_min, chunk.theta_max, chunk.phi_min, chunk.phi_max],
            max_range_m: chunk.max_range_m,
            link_index: chunk.link_index,
            rows: chunk.rows,
            cols: chunk.cols,
            sample: chunk.sample,
            stamp_ms: chunk.stamp_ms,
        }
    }
}

/// One sweep, organized `rows` × `cols` in row-major pulse order.
#[derive(Clone, Debug, PartialEq)]
pub struct LidarSweep {
    pub machine: String,
    pub link: String,
    pub sim_time_s: f64,
    /// The full capture sample when the sweep carries a capture, else the chunk's.
    pub sample: u64,
    pub rows: u32,
    pub cols: u32,
    pub max_range_m: f64,
    pub capture: Option<NativeLidarCapture>,
    /// Range per pulse; NaN where the pulse missed or never arrived.
    pub ranges: Vec<f32>,
    pub points: Vec<[f32; 3]>,
    /// Pulses delivered by some chunk.
    pub received: Vec<bool>,
    header: Header,
    delivered: u64,
}

impl LidarSweep {
    pub fn pulses(&self) -> usize {
        self.ranges.len()
    }

    pub fn complete(&self) -> bool {
        self.delivered == self.pulses() as u64
    }

    /// Organized indices no chunk delivered.
    pub fn missing(&self) -> Vec<u32> {
        (0..self.pulses() as u32)
            .filter(|&i| !self.received[i as usize])
            .collect()
    }

    pub fn hit(&self, pulse: usize) -> bool {
        self.received[pulse] && self.ranges[pulse].is_finite()
    }

    /// Turns hits nearer than `min_range_m` (returns off the carrier's own body)
    /// into invalid observations; pulse IDs and organization are unchanged.
    pub fn drop_near(&mut self, min_range_m: f32) -> usize {
        let mut dropped = 0;
        for pulse in 0..self.pulses() {
            if self.hit(pulse) && self.ranges[pulse] < min_range_m {
                self.ranges[pulse] = f32::NAN;
                self.points[pulse] = [f32::NAN; 3];
                dropped += 1;
            }
        }
        dropped
    }

    pub fn source_frame(&self) -> String {
        format!("{}/{}", self.machine, self.link)
    }

    pub fn schema() -> CloudSchema {
        let field = |name: &str, scalar, components, semantic, units: &str, nullable| Attribute {
            name: name.into(),
            scalar,
            components,
            semantic,
            units: units.into(),
            normalization: Normalization::None,
            nullable,
        };
        CloudSchema {
            revision: SCHEMA_REVISION,
            attributes: vec![
                field("position", ScalarType::F32, 3, AttributeSemantic::Position, "m", true),
                field("pulse", ScalarType::U64, 1, AttributeSemantic::SourceId, "", false),
                field("range", ScalarType::F32, 1, AttributeSemantic::Custom, "m", true),
                field("ring", ScalarType::U16, 1, AttributeSemantic::Ring, "", false),
            ],
        }
    }

    /// The scan descriptor for `placement`. Snapshot placement needs the
    /// sweep's capture transform; without one only the live link is honest.
    pub fn scan_descriptor(&self, scan_key: u64, placement: Placement) -> Result<ScanDescriptor, String> {
        let stamp = self.capture.as_ref().map(|c| c.transform.stamp.clone());
        let capture_transform = match placement {
            Placement::Live => None,
            Placement::Snapshot => Some(
                self.capture
                    .as_ref()
                    .ok_or("LiDAR sweep carries no capture transform; snapshot placement is unavailable")?
                    .transform
                    .clone(),
            ),
        };
        let scan = ScanDescriptor {
            revision: SCHEMA_REVISION,
            scan_key,
            source: Some(self.source_frame()),
            frame: DataFrame {
                frame: self.source_frame(),
                placement,
                origin_m: [0.0; 3],
                pose: RigidPose::default(),
                capture: stamp,
                capture_transform,
                ground_radius_m: None,
            },
            source_count: self.pulses() as u64,
            organization: Some(Organization {
                width: self.cols,
                height: self.rows,
            }),
        };
        scan.validate()?;
        Ok(scan)
    }

    /// Pages of at most [`MAX_PAGE_POINTS`] pulses in organized order.
    pub fn pages(&self) -> Vec<CloudPage> {
        let page = MAX_PAGE_POINTS as usize;
        (0..self.pulses().div_ceil(page))
            .map(|p| self.page(p * page..(p * page + page).min(self.pulses())))
            .collect()
    }

    fn page(&self, pulses: std::ops::Range<usize>) -> CloudPage {
        let count = pulses.len();
        let mask = || vec![0u8; count.div_ceil(8)];
        let (mut positions, mut position_valid) = (Vec::with_capacity(count * 12), mask());
        let (mut ranges, mut range_valid) = (Vec::with_capacity(count * 4), mask());
        let mut ids = Vec::with_capacity(count * 8);
        let mut rings = Vec::with_capacity(count * 2);
        for (row, pulse) in pulses.enumerate() {
            ids.extend((pulse as u64).to_le_bytes());
            rings.extend(((pulse / self.cols as usize) as u16).to_le_bytes());
            if self.hit(pulse) {
                positions.extend(self.points[pulse].iter().flat_map(|v| v.to_le_bytes()));
                ranges.extend(self.ranges[pulse].to_le_bytes());
                position_valid[row / 8] |= 1 << (row % 8);
                range_valid[row / 8] |= 1 << (row % 8);
            } else {
                positions.extend([0; 12]);
                ranges.extend([0; 4]);
            }
        }
        let column = |values, validity| ColumnData { values, validity };
        CloudPage {
            count: count as u32,
            columns: vec![
                column(positions, position_valid),
                column(ids, Vec::new()),
                column(ranges, range_valid),
                column(rings, Vec::new()),
            ],
        }
    }
}

/// Groups chunks into sweeps, bounded by [`LidarLimits`].
pub struct LidarAssembler {
    limits: LidarLimits,
    completion: Completion,
    open: Vec<LidarSweep>,
    /// Incomplete sweeps dropped since creation.
    pub abandoned: u64,
}

fn check_chunk(chunk: &LidarScan, limits: &LidarLimits) -> Result<(), String> {
    let pulses = u64::from(chunk.rows) * u64::from(chunk.cols);
    if pulses == 0 || pulses > limits.max_pulses {
        return Err(format!("LiDAR sweep of {pulses} pulses is outside 1..={}", limits.max_pulses));
    }
    if chunk.rows > u32::from(u16::MAX) + 1 {
        return Err("LiDAR sweep has more rings than a u16 ring index holds".into());
    }
    let count = chunk.pulse_count as usize;
    if chunk.ranges.len() != count || chunk.points.len() != count * 3 {
        return Err("LiDAR chunk arrays disagree with its pulse count".into());
    }
    if u64::from(chunk.pulse_offset) + u64::from(chunk.pulse_count) > pulses {
        return Err("LiDAR chunk runs past its sweep".into());
    }
    if !chunk.sim_time_s.is_finite() || chunk.sim_time_s < 0.0 {
        return Err("LiDAR chunk has no valid simulation time".into());
    }
    let mut hits = 0u32;
    for (range, point) in chunk.ranges.iter().zip(chunk.points.chunks_exact(3)) {
        if !range.is_finite() {
            continue;
        }
        if *range < 0.0 || f64::from(*range) > chunk.max_range_m * (1.0 + 1e-6) + 1e-6 {
            return Err(format!("LiDAR range {range} m is outside 0..={} m", chunk.max_range_m));
        }
        if point.iter().any(|v| !v.is_finite()) {
            return Err("LiDAR hit has a non-finite point".into());
        }
        hits += 1;
    }
    if hits != chunk.hits {
        return Err("LiDAR chunk hit count disagrees with its ranges".into());
    }
    Ok(())
}

impl LidarAssembler {
    pub fn new(limits: LidarLimits, completion: Completion) -> Self {
        Self {
            limits,
            completion,
            open: Vec::new(),
            abandoned: 0,
        }
    }

    pub fn open(&self) -> usize {
        self.open.len()
    }

    /// Adds one chunk received on `/machines/<machine>/sensors/<link>`. Returns
    /// the sweeps it finished, and in partial mode any it pushed out unfinished.
    pub fn push(&mut self, machine: &str, chunk: &LidarScan) -> Result<Vec<LidarSweep>, String> {
        check_chunk(chunk, &self.limits)?;
        let link = chunk.name();
        let capture = chunk.native_capture()?;
        if let Some(capture) = &capture
            && capture.machine != machine
        {
            return Err("LiDAR capture names another machine than its topic".into());
        }
        let header = Header::of(chunk);
        let at = self.open.iter().position(|s| {
            s.machine == machine
                && s.link == link
                && s.header.sample == header.sample
                && s.header.sim_time_s.to_bits() == header.sim_time_s.to_bits()
        });
        let mut out = Vec::new();
        let at = match at {
            Some(at) => {
                let sweep = &self.open[at];
                if sweep.header != header || sweep.capture != capture {
                    return Err("LiDAR chunk metadata disagrees with its sweep".into());
                }
                at
            }
            None => {
                if self.open.len() >= self.limits.max_open.max(1) {
                    let oldest = self.open.remove(0);
                    self.abandon(oldest, &mut out);
                }
                let pulses = (u64::from(chunk.rows) * u64::from(chunk.cols)) as usize;
                self.open.push(LidarSweep {
                    machine: machine.into(),
                    link,
                    sim_time_s: chunk.sim_time_s,
                    sample: capture
                        .as_ref()
                        .map_or(u64::from(chunk.sample), |c| c.transform.stamp.sample),
                    rows: chunk.rows,
                    cols: chunk.cols,
                    max_range_m: chunk.max_range_m,
                    capture,
                    ranges: vec![f32::NAN; pulses],
                    points: vec![[f32::NAN; 3]; pulses],
                    received: vec![false; pulses],
                    header,
                    delivered: 0,
                });
                self.open.len() - 1
            }
        };
        let sweep = &mut self.open[at];
        let span = chunk.pulse_offset as usize..(chunk.pulse_offset + chunk.pulse_count) as usize;
        if sweep.received[span.clone()].iter().any(|&r| r) {
            return Err("LiDAR chunk overlaps pulses already received".into());
        }
        for (i, pulse) in span.enumerate() {
            let range = chunk.ranges[i];
            sweep.received[pulse] = true;
            if range.is_finite() {
                sweep.ranges[pulse] = range;
                sweep.points[pulse] = [0, 1, 2].map(|k| chunk.points[3 * i + k]);
            }
        }
        sweep.delivered += u64::from(chunk.pulse_count);
        if sweep.complete() {
            out.push(self.open.remove(at));
        }
        Ok(out)
    }

    /// Ends every open sweep: partial mode returns them, otherwise they are dropped.
    pub fn flush(&mut self) -> Vec<LidarSweep> {
        let mut out = Vec::new();
        for sweep in std::mem::take(&mut self.open) {
            self.abandon(sweep, &mut out);
        }
        out
    }

    fn abandon(&mut self, sweep: LidarSweep, out: &mut Vec<LidarSweep>) {
        self.abandoned += 1;
        if self.completion == Completion::Partial {
            out.push(sweep);
        }
    }
}
