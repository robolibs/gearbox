//! Conversions between [`datapod`] spatial types (f64) and rapier / glam
//! (f32) types used by the physics engine.
//!
//! Gearbox speaks datapod at its public API surface and rapier internally;
//! every boundary crossing goes through these helpers so precision and
//! ordering rules live in one place.

use datapod::spatial::{Point, Pose, Quaternion, Size, Velocity};
use rapier3d::prelude::{Pose as RPose, Rot3, Vec3};

// -------- datapod → rapier --------

#[inline]
pub fn point_to_vec3(p: Point) -> Vec3 {
    Vec3::new(p.x as f32, p.y as f32, p.z as f32)
}

#[inline]
pub fn size_to_half_extents(s: Size) -> Vec3 {
    Vec3::new(
        (s.x * 0.5) as f32,
        (s.y * 0.5) as f32,
        (s.z * 0.5) as f32,
    )
}

#[inline]
pub fn size_to_vec3(s: Size) -> Vec3 {
    Vec3::new(s.x as f32, s.y as f32, s.z as f32)
}

#[inline]
pub fn quat_to_rot(q: Quaternion) -> Rot3 {
    // datapod: Quaternion(w, x, y, z). glam::Quat::from_xyzw(x, y, z, w).
    Rot3::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32).normalize()
}

#[inline]
pub fn pose_to_rpose(p: Pose) -> RPose {
    RPose::from_parts(point_to_vec3(p.point), quat_to_rot(p.rotation))
}

#[inline]
pub fn velocity_to_vec3(v: Velocity) -> Vec3 {
    Vec3::new(v.vx as f32, v.vy as f32, v.vz as f32)
}

// -------- rapier → datapod --------

#[inline]
pub fn vec3_to_point(v: Vec3) -> Point {
    Point::new(v.x as f64, v.y as f64, v.z as f64)
}

#[inline]
pub fn vec3_to_velocity(v: Vec3) -> Velocity {
    Velocity { vx: v.x as f64, vy: v.y as f64, vz: v.z as f64 }
}

#[inline]
pub fn rot_to_quat(r: Rot3) -> Quaternion {
    Quaternion::new(r.w as f64, r.x as f64, r.y as f64, r.z as f64)
}

#[inline]
pub fn rpose_to_pose(p: RPose) -> Pose {
    Pose {
        point: vec3_to_point(p.translation),
        rotation: rot_to_quat(p.rotation),
    }
}
