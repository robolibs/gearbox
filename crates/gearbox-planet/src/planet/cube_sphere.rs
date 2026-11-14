//! Cube-face parameterization of the sphere. CPU mirror of the WGSL mapping —
//! used only for node bounds / Aabbs, so it does not need to be bit-exact with
//! the shader (the shader is bit-exact with the bake shader, which is what
//! crack-freeness depends on).

use bevy::math::{Vec2, Vec3};

/// Per-face `[tangent, bitangent, normal]`, chosen right-handed
/// (`tangent x bitangent == normal`) so `d(pos)/du x d(pos)/dv` points outward.
/// Must match `face_basis` in the WGSL shaders.
pub const FACE_BASES: [[Vec3; 3]; 6] = [
    [Vec3::new(0.0, 0.0, -1.0), Vec3::Y, Vec3::X],
    [Vec3::new(0.0, 0.0, 1.0), Vec3::Y, Vec3::NEG_X],
    [Vec3::X, Vec3::new(0.0, 0.0, -1.0), Vec3::Y],
    [Vec3::X, Vec3::new(0.0, 0.0, 1.0), Vec3::NEG_Y],
    [Vec3::X, Vec3::Y, Vec3::Z],
    [Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z],
];

/// Face-local uv in [-1,1]^2 -> unit sphere direction.
pub fn face_uv_to_dir(face: u8, uv: Vec2) -> Vec3 {
    let [t, b, n] = FACE_BASES[face as usize];
    (n + uv.x * t + uv.y * b).normalize()
}
