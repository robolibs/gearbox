//! Conversions between [`datapod`] spatial types (f64) and rapier
//! (f64 via rapier3d-f64).
//!
//! Gearbox speaks datapod at its public API surface and rapier
//! internally; with rapier now in f64 most of these calls are pure
//! type-shuffling without precision loss.

use datapod::spatial::{Point, Pose, Quaternion, Size, Velocity};
use rapier3d::prelude::{Pose as RPose, Rot3, Vec3};

// -------- datapod → rapier --------

#[inline]
pub fn point_to_vec3(p: Point) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

#[inline]
pub fn size_to_half_extents(s: Size) -> Vec3 {
    Vec3::new(s.x * 0.5, s.y * 0.5, s.z * 0.5)
}

#[inline]
pub fn size_to_vec3(s: Size) -> Vec3 {
    Vec3::new(s.x, s.y, s.z)
}

#[inline]
pub fn quat_to_rot(q: Quaternion) -> Rot3 {
    // datapod: Quaternion(w, x, y, z). glam::DQuat::from_xyzw(x, y, z, w).
    Rot3::from_xyzw(q.x, q.y, q.z, q.w).normalize()
}

#[inline]
pub fn pose_to_rpose(p: Pose) -> RPose {
    RPose::from_parts(point_to_vec3(p.point), quat_to_rot(p.rotation))
}

#[inline]
pub fn velocity_to_vec3(v: Velocity) -> Vec3 {
    Vec3::new(v.vx, v.vy, v.vz)
}

// -------- rapier → datapod --------

#[inline]
pub fn vec3_to_point(v: Vec3) -> Point {
    Point::new(v.x, v.y, v.z)
}

#[inline]
pub fn vec3_to_velocity(v: Vec3) -> Velocity {
    Velocity {
        vx: v.x,
        vy: v.y,
        vz: v.z,
    }
}

#[inline]
pub fn rot_to_quat(r: Rot3) -> Quaternion {
    Quaternion::new(r.w, r.x, r.y, r.z)
}

#[inline]
pub fn rpose_to_pose(p: RPose) -> Pose {
    Pose {
        point: vec3_to_point(p.translation),
        rotation: rot_to_quat(p.rotation),
    }
}
