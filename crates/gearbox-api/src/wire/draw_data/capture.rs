use serde::{Deserialize, Serialize};

use super::CaptureTransform;
use super::frames::bounded_text;
use crate::host::MAX_PAYLOAD_BYTES;
use crate::wire::{LIDAR_CHUNK_PULSES, LidarScan, Props, pack};

pub const NATIVE_CAPTURE_PROP: &str = "capture.v1";
pub const MAX_NATIVE_CAPTURE_BYTES: usize = 2048;
pub const MAX_NATIVE_CAPTURE_PROPS_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureConvention {
    RightHandedZUpMetres,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLidarCapture {
    pub revision: u32,
    pub machine: String,
    pub link: String,
    pub convention: CaptureConvention,
    pub transform: CaptureTransform,
}

impl NativeLidarCapture {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision != 1 {
            return Err("unsupported native LiDAR capture revision".into());
        }
        bounded_text(&self.machine, 127, false)?;
        bounded_text(&self.link, 127, false)?;
        if self.machine.contains('/') || self.link.trim_matches('/') != self.link {
            return Err("native capture machine/link must form a canonical source frame".into());
        }
        self.transform.validate()?;
        if self.transform.source_frame != format!("{}/{}", self.machine, self.link) {
            return Err("native capture source frame disagrees with machine/link".into());
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, String> {
        self.validate()?;
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if json.len() > MAX_NATIVE_CAPTURE_BYTES {
            return Err("native capture exceeds 2 KiB".into());
        }
        Ok(json)
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.len() > MAX_NATIVE_CAPTURE_BYTES {
            return Err("native capture exceeds 2 KiB".into());
        }
        let capture: Self =
            serde_json::from_str(json).map_err(|e| format!("invalid native capture: {e}"))?;
        capture.validate()?;
        Ok(capture)
    }

    pub fn matches_chunk(&self, scan: &LidarScan) -> Result<(), String> {
        self.validate()?;
        if scan.props.len() > MAX_NATIVE_CAPTURE_PROPS_BYTES {
            return Err("native capture props exceed 4 KiB".into());
        }
        if self.transform.stamp.simulation_time_s != scan.sim_time_s
            || self.transform.stamp.sample as u32 != scan.sample
            || scan.name() != self.link
        {
            return Err("native capture does not match LiDAR chunk time/sample/link".into());
        }
        Ok(())
    }
}

impl LidarScan {
    pub fn native_capture(&self) -> Result<Option<NativeLidarCapture>, String> {
        let props = checked_props(&self.props)?;
        let Some(json) = props.get(NATIVE_CAPTURE_PROP) else {
            return Ok(None);
        };
        let capture = NativeLidarCapture::from_json(&json)?;
        capture.matches_chunk(self)?;
        Ok(Some(capture))
    }

    /// Replaces only the optional capture prop after validating the packed chunk size.
    pub fn set_native_capture(&mut self, capture: &NativeLidarCapture) -> Result<(), String> {
        let props = checked_props(&self.props)?;
        let count = self.pulse_count as usize;
        if count > LIDAR_CHUNK_PULSES
            || self.ranges.len() != count
            || self.points.len() != count * 3
            || u64::from(self.pulse_offset) + u64::from(self.pulse_count)
                > u64::from(self.rows) * u64::from(self.cols)
        {
            return Err("native capture requires a bounded, consistent LiDAR chunk".into());
        }
        capture.matches_chunk(self)?;
        let props = props
            .with(NATIVE_CAPTURE_PROP, &capture.to_json()?)
            .into_bytes();
        if props.len() > MAX_NATIVE_CAPTURE_PROPS_BYTES {
            return Err("native capture props exceed 4 KiB".into());
        }
        let previous = std::mem::replace(&mut self.props, props);
        if pack(self).wire().len() > MAX_PAYLOAD_BYTES {
            self.props = previous;
            return Err(
                "LiDAR chunk with capture exceeds host payload limit; reduce pulse count".into(),
            );
        }
        Ok(())
    }
}

/// Returns the largest supported pulse chunk that fits the supplied bounded props.
pub fn lidar_chunk_capacity(props: &[u8]) -> Result<usize, String> {
    checked_props(props)?;
    let mut sample = LidarScan {
        props: props.to_vec(),
        ..Default::default()
    };
    let header_bytes = pack(&sample).wire().len();
    let mut count = MAX_PAYLOAD_BYTES
        .saturating_sub(header_bytes)
        .checked_div(16)
        .unwrap()
        .min(LIDAR_CHUNK_PULSES);
    while count > 0 {
        sample.ranges.resize(count, 0.0);
        sample.points.resize(count * 3, 0.0);
        sample.pulse_count = count as u32;
        if pack(&sample).wire().len() <= MAX_PAYLOAD_BYTES {
            return Ok(count);
        }
        count -= 1;
    }
    Err("LiDAR props leave no room for a pulse".into())
}

fn checked_props(bytes: &[u8]) -> Result<Props, String> {
    if bytes.len() > MAX_NATIVE_CAPTURE_PROPS_BYTES {
        return Err("native capture props exceed 4 KiB".into());
    }
    if bytes.is_empty() {
        return Ok(Props::new());
    }
    let map = datapod::Map {
        data: bytes.to_vec(),
    };
    let count = map.try_size().map_err(|e| e.to_string())?;
    if count > 32 {
        return Err("native capture props exceed 32 fields".into());
    }
    for index in 0..count {
        std::str::from_utf8(map.try_key_at(index).map_err(|e| e.to_string())?)
            .map_err(|_| "invalid UTF-8 prop key")?;
        std::str::from_utf8(map.try_value_at(index).map_err(|e| e.to_string())?)
            .map_err(|_| "invalid UTF-8 prop value")?;
    }
    Ok(Props::from_bytes(bytes))
}
