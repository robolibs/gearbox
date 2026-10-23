//! Typed readers over authored opinions. A reader returns None when nothing
//! is authored, so callers attach the schema fallback with its provenance.

use bevy::math::{DQuat, DVec3};
use openusd::sdf::Value;

use super::scene::Scene;

pub fn value(scene: &Scene, prim: &str, name: &str) -> Option<Value> {
    crate::usd_ext::authored_value(scene.stage, &scene.sdf(prim)?, name)
}

pub fn f64_of(v: &Value) -> Option<f64> {
    Some(match v {
        Value::Double(d) => *d,
        Value::Float(f) => *f as f64,
        Value::Int(i) => *i as f64,
        Value::Int64(i) => *i as f64,
        Value::Uint(u) => *u as f64,
        Value::Half(h) => h.to_f64(),
        _ => return None,
    })
}

pub fn float(scene: &Scene, prim: &str, name: &str) -> Option<f64> {
    f64_of(&value(scene, prim, name)?)
}

pub fn int(scene: &Scene, prim: &str, name: &str) -> Option<Result<i64, String>> {
    Some(match value(scene, prim, name)? {
        Value::Int(i) => Ok(i as i64),
        Value::Int64(i) => Ok(i),
        Value::Uint(u) => Ok(u as i64),
        other => Err(format!("{other:?}")),
    })
}

pub fn boolean(scene: &Scene, prim: &str, name: &str) -> Option<bool> {
    match value(scene, prim, name)? {
        Value::Bool(b) => Some(b),
        _ => None,
    }
}

pub fn token(scene: &Scene, prim: &str, name: &str) -> Option<String> {
    match value(scene, prim, name)? {
        Value::Token(t) => Some(t.as_str().to_string()),
        Value::String(s) => Some(s),
        Value::AssetPath(a) => Some(a.as_str().to_string()),
        _ => None,
    }
}

pub fn tokens(scene: &Scene, prim: &str, name: &str) -> Option<Vec<String>> {
    match value(scene, prim, name)? {
        Value::TokenVec(v) => Some(v.iter().map(|t| t.as_str().to_string()).collect()),
        Value::StringVec(v) => Some(v),
        _ => None,
    }
}

pub fn bools(scene: &Scene, prim: &str, name: &str) -> Option<Vec<bool>> {
    match value(scene, prim, name)? {
        Value::BoolVec(v) => Some(v),
        _ => None,
    }
}

pub fn vec3(scene: &Scene, prim: &str, name: &str) -> Option<DVec3> {
    Some(match value(scene, prim, name)? {
        Value::Vec3f(v) => DVec3::new(v.x as f64, v.y as f64, v.z as f64),
        Value::Vec3d(v) => DVec3::new(v.x, v.y, v.z),
        _ => return None,
    })
}

/// A quaternion; an authored zero quaternion is reported as invalid.
pub fn quat(scene: &Scene, prim: &str, name: &str) -> Option<Result<DQuat, String>> {
    let v = value(scene, prim, name)?;
    Some(super::xform::quat(&v).ok_or_else(|| format!("{name} is not a usable quaternion: {v:?}")))
}

pub fn targets(scene: &Scene, prim: &str, name: &str) -> Option<Vec<String>> {
    let p = scene.stage.prim(&scene.sdf(prim)?).ok()?;
    let rel = p.relationship(name);
    if !rel.has_authored_targets().unwrap_or(false) {
        return None;
    }
    Some(rel.targets().unwrap_or_default().into_iter().map(|t| t.as_str().to_string()).collect())
}

pub fn first_target(scene: &Scene, prim: &str, name: &str) -> Option<String> {
    targets(scene, prim, name).and_then(|t| t.into_iter().next())
}

pub fn property_names(scene: &Scene, prim: &str) -> Vec<String> {
    use crate::usd_ext::StageExt;
    scene.sdf(prim).and_then(|p| scene.stage.prim_properties(&p).ok()).unwrap_or_default()
}
