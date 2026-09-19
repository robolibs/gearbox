//! A planet as tangent sites.
//!
//! The planet frame has its origin at the planet's centre. A [`SiteFrame`] is
//! a flat, Y-up frame touching the surface where `up` leaves it; everything on
//! the ground is simulated and drawn in such a frame. The terrain is one
//! function of the place on the planet, so any two sites agree on the land
//! they share, and a site's ground falls away with the planet's curve.

mod noise;
mod terrain;

use bevy::math::{DQuat, DVec3};

pub use terrain::{RELIEF_M, Terrain};

pub const PLANET_RADIUS_M: f64 = 6_371_000.0;
/// A site serves the land this far from where it touches the planet; past it
/// the tangent frame leans too far from the true vertical.
pub const SITE_REACH_M: f64 = 25_000.0;
/// Sites sit this far apart along X in the one physics world they share.
pub const SITE_SPACING_M: f64 = 1.0e6;

/// A flat frame touching the planet where `up` leaves its centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SiteFrame {
    pub up: DVec3,
    pub rotation: DQuat,
}

impl SiteFrame {
    /// The frame at `up`, turned from the planet frame's Y by the shortest arc.
    pub fn at(up: DVec3) -> Self {
        let up = up.normalize();
        Self { up, rotation: DQuat::from_rotation_arc(DVec3::Y, up) }
    }

    /// Where the frame touches the planet, in the planet frame.
    pub fn origin(&self) -> DVec3 {
        self.up * PLANET_RADIUS_M
    }

    pub fn to_planet(&self, local: DVec3) -> DVec3 {
        self.origin() + self.rotation * local
    }

    pub fn from_planet(&self, planet: DVec3) -> DVec3 {
        self.rotation.inverse() * (planet - self.origin())
    }

    /// The direction from the planet's centre through the ground under a local place.
    pub fn direction(&self, x: f64, z: f64) -> DVec3 {
        self.to_planet(DVec3::new(x, 0.0, z)).normalize()
    }

    /// The frame reached from this one by going `distance` metres along the
    /// ground on `bearing` radians, turning from local -Z towards local +X.
    pub fn travelled(&self, bearing: f64, distance: f64) -> Self {
        let heading = self.rotation * DVec3::new(bearing.sin(), 0.0, -bearing.cos());
        let angle = distance / PLANET_RADIUS_M;
        Self::at(self.up * angle.cos() + heading * angle.sin())
    }

    /// Great-circle distance to the ground under another frame.
    pub fn ground_distance(&self, other: DVec3) -> f64 {
        self.up.dot(other.normalize()).clamp(-1.0, 1.0).acos() * PLANET_RADIUS_M
    }
}

/// The region of the shared physics world that belongs to site `index`.
pub fn physics_offset(index: usize) -> DVec3 {
    DVec3::X * (index as f64 * SITE_SPACING_M)
}

/// The site whose region holds physics coordinate `x`.
pub fn site_of_physics(x: f64) -> usize {
    (x / SITE_SPACING_M).round().max(0.0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_frame_is_the_planet_frame_lifted_to_the_surface() {
        let home = SiteFrame::at(DVec3::Y);
        let p = home.to_planet(DVec3::new(3.0, 2.0, -5.0));
        assert!((p - DVec3::new(3.0, PLANET_RADIUS_M + 2.0, -5.0)).length() < 1e-6);
    }

    #[test]
    fn frames_round_trip_anywhere() {
        let far = SiteFrame::at(DVec3::new(0.3, -0.8, 0.5));
        let local = DVec3::new(1234.5, 6.7, -890.1);
        assert!((far.from_planet(far.to_planet(local)) - local).length() < 1e-6);
    }

    #[test]
    fn travelling_half_the_circumference_reaches_the_far_side() {
        let home = SiteFrame::at(DVec3::Y);
        let far = home.travelled(1.0, std::f64::consts::PI * PLANET_RADIUS_M * 0.999);
        assert!(far.up.dot(DVec3::Y) < -0.99);
        assert!((home.ground_distance(far.up) / 1000.0 - 20_000.0).abs() < 50.0);
    }

    #[test]
    fn physics_regions_map_back_to_their_site() {
        for index in [0usize, 1, 7, 300] {
            let inside = physics_offset(index) + DVec3::new(-30_000.0, 5.0, 40_000.0);
            assert_eq!(site_of_physics(inside.x), index);
        }
    }
}
