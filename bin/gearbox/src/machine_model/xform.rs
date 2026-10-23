//! Composed prim transforms from `xformOpOrder`: every op type, `!invert!`
//! pivots and `!resetXformStack!`, and the split of a matrix into a rigid
//! part plus scale with shear and reflection detection.

use bevy::math::{DMat3, DMat4, DQuat, DVec3};
use openusd::sdf::{Path as SdfPath, Value};
use openusd::usd::Stage;

use super::model::RigidTransform;

/// A prim's own transform and whether it resets the parent stack.
pub struct LocalXform {
    pub matrix: DMat4,
    pub resets: bool,
    pub unsupported: Vec<String>,
}

fn vec3(value: &Value) -> Option<DVec3> {
    Some(match value {
        Value::Vec3d(v) => DVec3::new(v.x, v.y, v.z),
        Value::Vec3f(v) => DVec3::new(v.x as f64, v.y as f64, v.z as f64),
        Value::Double(v) => DVec3::splat(*v),
        Value::Float(v) => DVec3::splat(*v as f64),
        _ => return None,
    })
}

fn scalar(value: &Value) -> Option<f64> {
    Some(match value {
        Value::Double(v) => *v,
        Value::Float(v) => *v as f64,
        Value::Int(v) => *v as f64,
        _ => return None,
    })
}

pub fn quat(value: &Value) -> Option<DQuat> {
    let q = match value {
        Value::Quatd(q) => DQuat::from_xyzw(q.x, q.y, q.z, q.w),
        Value::Quatf(q) => DQuat::from_xyzw(q.x as f64, q.y as f64, q.z as f64, q.w as f64),
        _ => return None,
    };
    (q.length_squared() > 1e-12).then(|| q.normalize())
}

/// Rotation about the listed axes applied first to last (extrinsic), as
/// USD's `rotateXYZ` family defines.
fn ordered_rotation(order: &str, degrees: DVec3) -> Option<DQuat> {
    let mut q = DQuat::IDENTITY;
    for axis in order.chars() {
        let step = match axis {
            'X' => DQuat::from_rotation_x(degrees.x.to_radians()),
            'Y' => DQuat::from_rotation_y(degrees.y.to_radians()),
            'Z' => DQuat::from_rotation_z(degrees.z.to_radians()),
            _ => return None,
        };
        q = step * q;
    }
    Some(q)
}

fn op_matrix(op: &str, value: &Value) -> Option<DMat4> {
    let mut parts = op.split(':');
    let (Some("xformOp"), Some(kind)) = (parts.next(), parts.next()) else {
        return None;
    };
    Some(match kind {
        "translate" => DMat4::from_translation(vec3(value)?),
        "scale" => DMat4::from_scale(vec3(value)?),
        "orient" => DMat4::from_quat(quat(value)?),
        "rotateX" => DMat4::from_rotation_x(scalar(value)?.to_radians()),
        "rotateY" => DMat4::from_rotation_y(scalar(value)?.to_radians()),
        "rotateZ" => DMat4::from_rotation_z(scalar(value)?.to_radians()),
        "transform" => match value {
            Value::Matrix4d(m) => DMat4::from_cols_array(&m.0),
            _ => return None,
        },
        rotate if rotate.len() == 9 && rotate.starts_with("rotate") => {
            DMat4::from_quat(ordered_rotation(&rotate[6..], vec3(value)?)?)
        }
        _ => return None,
    })
}

/// The prim's local transform from its authored `xformOpOrder`.
pub fn local_xform(stage: &Stage, prim: &SdfPath) -> LocalXform {
    let order = match crate::usd_ext::authored_value(stage, prim, "xformOpOrder") {
        Some(Value::TokenVec(v)) => v.iter().map(|t| t.as_str().to_string()).collect(),
        Some(Value::StringVec(v)) => v,
        _ => Vec::new(),
    };
    let mut matrix = DMat4::IDENTITY;
    let mut resets = false;
    let mut unsupported = Vec::new();
    for op in order {
        if op == "!resetXformStack!" {
            resets = true;
            matrix = DMat4::IDENTITY;
            continue;
        }
        let inverted = op.starts_with("!invert!");
        let name = op.trim_start_matches("!invert!");
        let Some(value) = crate::usd_ext::authored_value(stage, prim, name) else {
            unsupported.push(format!("{name}: no value"));
            continue;
        };
        match op_matrix(name, &value) {
            Some(step) => matrix *= if inverted { step.inverse() } else { step },
            None => unsupported.push(name.to_string()),
        }
    }
    LocalXform { matrix, resets, unsupported }
}

/// Composed local-to-world matrix of `prim` in stage axes and units.
pub fn world_matrix(stage: &Stage, prim: &str) -> (DMat4, Vec<String>) {
    let mut chain = Vec::new();
    let mut current = prim.to_string();
    loop {
        chain.push(current.clone());
        match current.rfind('/') {
            Some(0) | None => break,
            Some(pos) => current.truncate(pos),
        }
    }
    let mut matrix = DMat4::IDENTITY;
    let mut unsupported = Vec::new();
    for path in chain.iter().rev() {
        let Ok(sdf) = openusd::sdf::path(path) else { continue };
        let local = local_xform(stage, &sdf);
        unsupported.extend(local.unsupported.into_iter().map(|u| format!("{path}: {u}")));
        matrix = if local.resets { local.matrix } else { matrix * local.matrix };
    }
    (matrix, unsupported)
}

/// Largest relative spread of the axis scales still read as one uniform scale.
pub const UNIFORM_SCALE_TOLERANCE: f64 = 1e-3;

/// A matrix split into a rigid transform and the scale it carried.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decomposed {
    pub rigid: RigidTransform,
    pub scale: DVec3,
    pub shear: bool,
    pub reflection: bool,
}

impl Decomposed {
    pub fn uniform_scale(&self) -> Option<f64> {
        let s = self.scale;
        let mean = (s.x + s.y + s.z) / 3.0;
        (mean > 0.0 && (s - DVec3::splat(mean)).abs().max_element() <= UNIFORM_SCALE_TOLERANCE * mean).then_some(mean)
    }
}

pub fn decompose(m: &DMat4) -> Decomposed {
    let basis = DMat3::from_cols(m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate());
    let scale = DVec3::new(basis.x_axis.length(), basis.y_axis.length(), basis.z_axis.length());
    let det = basis.determinant();
    let reflection = det < 0.0;
    let unit = |v: DVec3| v.normalize_or_zero();
    let (x, y, z) = (unit(basis.x_axis), unit(basis.y_axis), unit(basis.z_axis));
    let shear = x.dot(y).abs() > 1e-6 || y.dot(z).abs() > 1e-6 || z.dot(x).abs() > 1e-6;
    let z_fixed = if reflection { -z } else { z };
    let rotation = if scale.min_element() > 1e-12 {
        DQuat::from_mat3(&DMat3::from_cols(x, y, z_fixed)).normalize()
    } else {
        DQuat::IDENTITY
    };
    Decomposed {
        rigid: RigidTransform::new(m.w_axis.truncate(), rotation),
        scale,
        shear,
        reflection,
    }
}

/// Rotation that takes the stage's axes to the normalized Z-up world.
pub fn stage_to_world(up: super::model::UpAxis) -> DQuat {
    match up {
        super::model::UpAxis::Z => DQuat::IDENTITY,
        super::model::UpAxis::Y => DQuat::from_rotation_x(std::f64::consts::FRAC_PI_2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_xyz_applies_x_first_then_y_then_z() {
        let q = ordered_rotation("XYZ", DVec3::new(90.0, 90.0, 0.0)).unwrap();
        assert!((q * DVec3::Y - DVec3::X).length() < 1e-9, "{:?}", q * DVec3::Y);
        let zyx = ordered_rotation("ZYX", DVec3::new(0.0, 90.0, 90.0)).unwrap();
        assert!((zyx * DVec3::Z - DVec3::X).length() < 1e-9, "{:?}", zyx * DVec3::Z);
        let xyz = ordered_rotation("XYZ", DVec3::new(0.0, 90.0, 90.0)).unwrap();
        assert!((xyz * DVec3::Z - DVec3::Y).length() < 1e-9, "{:?}", xyz * DVec3::Z);
    }

    #[test]
    fn decompose_flags_shear_reflection_and_nonuniform_scale() {
        let rigid = DMat4::from_rotation_translation(DQuat::from_rotation_z(0.4), DVec3::new(1.0, 2.0, 3.0));
        let d = decompose(&(rigid * DMat4::from_scale(DVec3::splat(2.0))));
        assert_eq!(d.uniform_scale(), Some(2.0));
        assert!(!d.shear && !d.reflection);
        assert!(d.rigid.rotation.angle_between(DQuat::from_rotation_z(0.4)) < 1e-9);

        let d = decompose(&DMat4::from_scale(DVec3::new(1.0, 2.0, 1.0)));
        assert_eq!(d.uniform_scale(), None);

        let d = decompose(&DMat4::from_scale(DVec3::new(1.0, 1.0, -1.0)));
        assert!(d.reflection);

        let mut sheared = DMat4::IDENTITY;
        sheared.y_axis.x = 0.5;
        assert!(decompose(&sheared).shear);
    }
}
