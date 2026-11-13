use serde::{Deserialize, Serialize};

use crate::wire::DrawFrame;

pub(crate) fn bounded_text(value: &str, max: usize, empty: bool) -> Result<(), String> {
    if (!empty && value.is_empty())
        || value.len() > max
        || value.chars().any(char::is_control)
        || value.trim() != value
    {
        return Err(format!(
            "text must fit {max} UTF-8 bytes without control or outer whitespace"
        ));
    }
    Ok(())
}

pub(crate) fn validate_quaternion(value: [f64; 4]) -> Result<(), String> {
    if value.iter().any(|v| !v.is_finite())
        || (value.iter().map(|v| v * v).sum::<f64>() - 1.0).abs() > 1e-5
    {
        return Err("rotation must be a finite unit quaternion in w,x,y,z order".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RigidPose {
    pub translation_m: [f64; 3],
    pub rotation_wxyz: [f64; 4],
}

impl Default for RigidPose {
    fn default() -> Self {
        Self {
            translation_m: [0.0; 3],
            rotation_wxyz: [1.0, 0.0, 0.0, 0.0],
        }
    }
}

impl RigidPose {
    pub fn validate(&self) -> Result<(), String> {
        if self.translation_m.iter().any(|v| !v.is_finite()) {
            return Err("pose translation must be finite metres".into());
        }
        validate_quaternion(self.rotation_wxyz)
    }

    pub fn transform_vector(&self, vector: [f64; 3]) -> Result<[f64; 3], String> {
        self.validate()?;
        finite_point(vector)?;
        let [w, x, y, z] = self.rotation_wxyz;
        let q = [x, y, z];
        let cross = |a: [f64; 3], b: [f64; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let first = cross(q, vector);
        let second = cross(q, std::array::from_fn(|i| first[i] + w * vector[i]));
        let scale = 2.0 / (w * w + x * x + y * y + z * z);
        finite_point(std::array::from_fn(|i| vector[i] + scale * second[i]))
    }

    pub fn transform_point(&self, point: [f64; 3]) -> Result<[f64; 3], String> {
        let rotated = self.transform_vector(point)?;
        finite_point(std::array::from_fn(|i| rotated[i] + self.translation_m[i]))
    }
}

fn finite_point(point: [f64; 3]) -> Result<[f64; 3], String> {
    if point.iter().any(|v| !v.is_finite()) {
        Err("transformed coordinates must remain finite".into())
    } else {
        Ok(point)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureStamp {
    pub simulation_time_s: f64,
    pub source_time_ns: Option<u64>,
    pub source_clock: Option<String>,
    pub sample: u64,
    pub scene_epoch: u64,
}

impl CaptureStamp {
    pub fn validate(&self) -> Result<(), String> {
        if !self.simulation_time_s.is_finite()
            || self.simulation_time_s < 0.0
            || self.scene_epoch == 0
        {
            return Err(
                "capture requires finite nonnegative sim time and a nonzero scene epoch".into(),
            );
        }
        match (&self.source_time_ns, &self.source_clock) {
            (Some(_), Some(clock)) => bounded_text(clock, 63, false),
            (None, None) => Ok(()),
            _ => Err("source timestamp and clock must be specified together".into()),
        }
    }

    pub fn source_sample_ns(&self, offset_ns: u64) -> Result<u64, String> {
        self.validate()?;
        self.source_time_ns
            .ok_or("source clock is unavailable")?
            .checked_add(offset_ns)
            .ok_or_else(|| "source acquisition time overflow".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatumSnapshot {
    pub origin_ecef_m: [f64; 3],
    pub axes_to_ecef_wxyz: [f64; 4],
}

impl DatumSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.origin_ecef_m.iter().any(|v| !v.is_finite()) {
            return Err("capture datum must have finite ECEF coordinates".into());
        }
        validate_quaternion(self.axes_to_ecef_wxyz)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureTransform {
    pub revision: u32,
    pub source_frame: String,
    pub target_frame: String,
    pub stamp: CaptureStamp,
    pub pose: RigidPose,
    pub datum: Option<DatumSnapshot>,
}

impl CaptureTransform {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision != 1 {
            return Err("unsupported capture transform revision".into());
        }
        bounded_text(&self.source_frame, 255, false)?;
        bounded_text(&self.target_frame, 255, false)?;
        if matches!(DrawFrame::parse(&self.target_frame), DrawFrame::Link(..)) {
            return Err("capture target must be a world or odom datum, not a live link".into());
        }
        self.stamp.validate()?;
        self.pose.validate()?;
        self.datum
            .as_ref()
            .ok_or("capture requires an anchored target datum")?
            .validate()?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Live,
    Snapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataFrame {
    pub frame: String,
    pub placement: Placement,
    pub origin_m: [f64; 3],
    pub pose: RigidPose,
    pub capture: Option<CaptureStamp>,
    pub capture_transform: Option<CaptureTransform>,
    /// When set, the origin's z is metres above the highest terrain within this
    /// radius of the origin, so a rigid dataset never sinks into a slope. Only for
    /// live world frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground_radius_m: Option<f64>,
}

impl Default for DataFrame {
    fn default() -> Self {
        Self {
            frame: "world".into(),
            placement: Placement::Live,
            origin_m: [0.0; 3],
            pose: RigidPose::default(),
            capture: None,
            capture_transform: None,
            ground_radius_m: None,
        }
    }
}

impl DataFrame {
    pub fn validate(&self) -> Result<(), String> {
        bounded_text(&self.frame, 255, false)?;
        self.pose.validate()?;
        if let Some(radius) = self.ground_radius_m
            && (!(0.0..=200.0).contains(&radius) || self.frame != "world" || self.placement != Placement::Live)
        {
            return Err("ground-relative origins need a live world frame and a 0..200 m radius".into());
        }
        if self.origin_m.iter().any(|v| !v.is_finite()) {
            return Err("coordinate origin must be finite metres".into());
        }
        if let Some(stamp) = &self.capture {
            stamp.validate()?;
        }
        match (self.placement, &self.capture_transform) {
            (Placement::Live, None) => Ok(()),
            (Placement::Snapshot, Some(transform)) => {
                transform.validate()?;
                if transform.source_frame != self.frame || self.capture.as_ref() != Some(&transform.stamp) {
                    return Err("snapshot pose must match the source frame and exact capture stamp".into());
                }
                Ok(())
            }
            _ => Err("snapshot placement requires a capture transform; live placement must not carry one".into()),
        }
    }

    /// Dataset-local origins precede the dataset's rigid pose.
    pub fn source_point(
        &self,
        page_origin_m: [f64; 3],
        offset_m: [f64; 3],
    ) -> Result<[f64; 3], String> {
        self.validate()?;
        finite_point(page_origin_m)?;
        finite_point(offset_m)?;
        let local = std::array::from_fn(|i| self.origin_m[i] + page_origin_m[i] + offset_m[i]);
        self.pose.transform_point(local)
    }

    pub fn target_point(
        &self,
        page_origin_m: [f64; 3],
        offset_m: [f64; 3],
        live: Option<&RigidPose>,
    ) -> Result<[f64; 3], String> {
        let source = self.source_point(page_origin_m, offset_m)?;
        let transform = match self.placement {
            Placement::Live => live.ok_or("live frame transform is unavailable")?,
            Placement::Snapshot => {
                &self
                    .capture_transform
                    .as_ref()
                    .ok_or("capture transform is unavailable")?
                    .pose
            }
        };
        transform.transform_point(source)
    }

    pub fn captured_ecef_point(
        &self,
        page_origin_m: [f64; 3],
        offset_m: [f64; 3],
    ) -> Result<[f64; 3], String> {
        if self.placement != Placement::Snapshot {
            return Err("ECEF capture query requires snapshot placement".into());
        }
        let target = self.target_point(page_origin_m, offset_m, None)?;
        let datum = self
            .capture_transform
            .as_ref()
            .and_then(|t| t.datum.as_ref())
            .ok_or("capture datum is unavailable")?;
        RigidPose {
            translation_m: datum.origin_ecef_m,
            rotation_wxyz: datum.axes_to_ecef_wxyz,
        }
        .transform_point(target)
    }
}
