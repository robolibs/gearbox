//! The land: one height for every place on the planet.

use bevy::math::DVec3;

use crate::{PLANET_RADIUS_M, SiteFrame, noise};

const FLAT_RADIUS_M: f64 = 30.0;
const GENTLE_RADIUS_M: f64 = 70.0;
const RELIEF_FROM_M: f64 = 400.0;
const RELIEF_FULL_M: f64 = 2_000.0;
/// Peak-to-trough height of the broad relief away from the home field.
pub const RELIEF_M: f32 = 160.0;

fn smooth(t: f64) -> f32 {
    let t = t.clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

/// The planet's land. It is level where `home` touches it, and gentle around
/// that, so a field can be laid there; the broad relief rises farther out.
#[derive(Clone, Copy, Debug)]
pub struct Terrain {
    home: DVec3,
    home_raw: f32,
}

impl Terrain {
    pub fn new(home: &SiteFrame) -> Self {
        let home = home.origin();
        Self { home, home_raw: Self::meadow(home) }
    }

    fn meadow(p: DVec3) -> f32 {
        (noise::fbm(p * 0.0045, 5) - 0.5) * 7.0
            + (noise::fbm(p * 0.035 + DVec3::splat(11.0), 3) - 0.5) * 0.5
            + (noise::fbm(p * 0.0011 + DVec3::splat(53.0), 3) - 0.5) * 34.0
    }

    /// Height above the sphere in the direction `direction` from the centre.
    pub fn height(&self, direction: DVec3) -> f32 {
        let p = direction * PLANET_RADIUS_M;
        let from_home = (p - self.home).length();
        let level = smooth((from_home - FLAT_RADIUS_M) / (GENTLE_RADIUS_M - FLAT_RADIUS_M));
        let relief = smooth((from_home - RELIEF_FROM_M) / (RELIEF_FULL_M - RELIEF_FROM_M))
            * (noise::fbm(p * 0.00045 + DVec3::splat(97.0), 4) - 0.5)
            * RELIEF_M;
        (Self::meadow(p) - self.home_raw) * level + relief
    }

    /// Height of the land in a site's own frame at its local `x`, `z`: the
    /// land's height less what the planet's curve has dropped by there.
    pub fn local_height(&self, site: &SiteFrame, x: f64, z: f64) -> f32 {
        let tangent = site.to_planet(DVec3::new(x, 0.0, z));
        let length = tangent.length();
        let height = self.height(tangent / length) as f64;
        ((PLANET_RADIUS_M + height) * (PLANET_RADIUS_M / length) - PLANET_RADIUS_M) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_field_is_level_where_it_starts() {
        let home = SiteFrame::at(DVec3::Y);
        let land = Terrain::new(&home);
        assert!(land.local_height(&home, 0.0, 0.0).abs() < 1e-3);
        assert!(land.local_height(&home, 12.0, -9.0).abs() < 1e-3);
    }

    #[test]
    fn the_ground_falls_away_with_the_curve() {
        let home = SiteFrame::at(DVec3::Y);
        let land = Terrain::new(&home);
        let drop = 20_000.0f64.powi(2) / (2.0 * PLANET_RADIUS_M);
        let height = land.local_height(&home, 20_000.0, 0.0) as f64;
        assert!((height + drop).abs() < RELIEF_M as f64, "{height} vs {drop}");
    }

    #[test]
    fn two_sites_agree_on_the_land_they_share() {
        let home = SiteFrame::at(DVec3::Y);
        let land = Terrain::new(&home);
        let near = home.travelled(0.7, 9_000.0);
        let spot = near.to_planet(DVec3::new(150.0, 0.0, -80.0)).normalize();
        let seen_from_home = home.from_planet(spot * PLANET_RADIUS_M);
        let a = land.height(near.direction(150.0, -80.0));
        let b = land.height(home.direction(seen_from_home.x, seen_from_home.z));
        assert!((a - b).abs() < 0.05, "{a} vs {b}");
    }
}
