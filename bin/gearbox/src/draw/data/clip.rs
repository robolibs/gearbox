use bevy::math::{DMat3, DQuat, DVec3, Mat3, Mat4, Vec3, Vec4};
use gearbox_api::{
    draw_data::StorageError,
    wire::draw_data::{ClipBounds, DataFrame},
};

use super::cloud::{Geometry, MAX_LOCAL_ERROR_M, invalid};

#[cfg(test)]
#[path = "clip_tests.rs"]
mod tests;

/// Clip mode in `min.w`: 0 keeps all, 1 keeps the inside, 2 keeps none, 3 keeps the outside.
pub(super) const INVERTED: f32 = 3.0;

#[derive(Clone, Copy)]
pub(super) struct Clip {
    pub bounds: Option<ClipBounds>,
    pub rotation: Mat4,
    pub min: Vec4,
    pub max: Vec4,
    pub selected: u64,
}

pub(super) fn rotated(rotation: Mat3, point: Vec3) -> Vec3 {
    let dot = |axis: usize| {
        rotation.x_axis[axis].mul_add(
            point.x,
            rotation.y_axis[axis].mul_add(point.y, rotation.z_axis[axis] * point.z),
        )
    };
    Vec3::new(dot(0), dot(1), dot(2))
}

impl Clip {
    pub fn accepts(&self, geometry: &Geometry, ordinal: u32) -> bool {
        if self.min.w == 0.0 {
            return true;
        }
        if self.min.w == 2.0 {
            return false;
        }
        let Some(point) = geometry.local_point(ordinal) else {
            return false;
        };
        let point = Vec3::from_array(point);
        let value = rotated(Mat3::from_mat4(self.rotation), point);
        let inside = (0..3).all(|axis| value[axis] >= self.min[axis] && value[axis] <= self.max[axis]);
        inside != (self.min.w == INVERTED)
    }

    pub fn prepare(
        geometry: &Geometry,
        frame: &DataFrame,
        bounds: Option<ClipBounds>,
    ) -> Result<Self, StorageError> {
        let mut result = Self {
            bounds,
            rotation: Mat4::IDENTITY,
            min: Vec4::ZERO,
            max: Vec4::ZERO,
            selected: geometry.positions.values.len() as u64,
        };
        let Some(bounds) = bounds else {
            return Ok(result);
        };
        bounds.validate().map_err(invalid)?;
        frame.validate().map_err(invalid)?;
        if result.selected == 0 {
            return Ok(result);
        }
        let quat = |q: [f64; 4]| DQuat::from_xyzw(q[1], q[2], q[3], q[0]);
        let to_box = bounds.rotation_wxyz.map_or(DQuat::IDENTITY, |q| quat(q).inverse());
        let pose = quat(frame.pose.rotation_wxyz);
        let rotation64 = DMat3::from_quat(to_box * pose);
        let rotation = rotation64.as_mat3();
        let anchor = to_box
            * (pose
                * (DVec3::from_array(frame.origin_m)
                    + DVec3::from_array(geometry.descriptor().origin_m)
                    + DVec3::from_array(geometry.rebase_m))
                + DVec3::from_array(frame.pose.translation_m));
        if !anchor.is_finite() {
            return Err(invalid("nonfinite source-frame clipping origin"));
        }
        let low = DVec3::from_array(bounds.min_m) - anchor;
        let high = DVec3::from_array(bounds.max_m) - anchor;
        let mut extent_min = Vec3::splat(f32::MAX);
        let mut extent_max = Vec3::splat(-f32::MAX);
        for point in &geometry.positions.values {
            let point = Vec3::from_array(point.xyz);
            let value = rotated(rotation, point);
            if !value.is_finite()
                || (value.as_dvec3() - rotation64 * point.as_dvec3())
                    .abs()
                    .max_element()
                    > MAX_LOCAL_ERROR_M
            {
                return Err(invalid(
                    "clip rotation exceeds finite one-millimetre GPU precision",
                ));
            }
            extent_min = extent_min.min(value);
            extent_max = extent_max.max(value);
        }
        if (0..3).any(|axis| {
            low[axis] > f64::from(extent_max[axis]) || high[axis] < f64::from(extent_min[axis])
        }) {
            if bounds.invert {
                return Ok(result);
            }
            result.min.w = 2.0;
            result.selected = 0;
            return Ok(result);
        }
        let boundary = |value: f64, axis: usize| -> Result<f32, StorageError> {
            let bounded = value.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
            if value >= f64::from(extent_min[axis])
                && value <= f64::from(extent_max[axis])
                && (f64::from(bounded) - value).abs() > MAX_LOCAL_ERROR_M
            {
                return Err(invalid(
                    "clip boundary exceeds one-millimetre GPU precision",
                ));
            }
            Ok(bounded)
        };
        result.rotation = Mat4::from_mat3(rotation);
        result.min.w = if bounds.invert { INVERTED } else { 1.0 };
        for axis in 0..3 {
            result.min[axis] = boundary(low[axis], axis)?;
            result.max[axis] = boundary(high[axis], axis)?;
        }
        result.selected = geometry
            .positions
            .values
            .iter()
            .filter(|point| {
                let value = rotated(rotation, Vec3::from_array(point.xyz));
                let inside = (0..3)
                    .all(|axis| value[axis] >= result.min[axis] && value[axis] <= result.max[axis]);
                inside != bounds.invert
            })
            .count() as u64;
        if result.selected == 0 {
            result.min.w = 2.0;
        }
        Ok(result)
    }
}
