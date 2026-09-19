//! The land: one height above the ellipsoid for every place on Earth.

use bevy::math::DVec3;

use crate::{Datum, Geodetic, noise};

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

/// The land. It is level where `home` is anchored, and gentle around that, so
/// a field can be laid there; the broad relief rises farther out.
#[derive(Clone, Copy, Debug)]
pub struct Terrain {
    home: DVec3,
    home_raw: f32,
}

impl Terrain {
    pub fn new(home: &Datum) -> Self {
        Self { home: home.origin, home_raw: Self::meadow(home.origin) }
    }

    fn meadow(p: DVec3) -> f32 {
        (noise::fbm(p * 0.0045, 5) - 0.5) * 7.0
            + (noise::fbm(p * 0.035 + DVec3::splat(11.0), 3) - 0.5) * 0.5
            + (noise::fbm(p * 0.0011 + DVec3::splat(53.0), 3) - 0.5) * 34.0
    }

    /// Height above the ellipsoid of the land over `ground`, an ECEF place on it.
    pub fn height(&self, ground: DVec3) -> f32 {
        let from_home = (ground - self.home).length();
        let level = smooth((from_home - FLAT_RADIUS_M) / (GENTLE_RADIUS_M - FLAT_RADIUS_M));
        let relief = smooth((from_home - RELIEF_FROM_M) / (RELIEF_FULL_M - RELIEF_FROM_M))
            * (noise::fbm(ground * 0.00045 + DVec3::splat(97.0), 4) - 0.5)
            * RELIEF_M;
        (Self::meadow(ground) - self.home_raw) * level + relief
    }

    /// Height of the land in a datum's frame at its `x`, `z`: the land's
    /// altitude there, less the altitude the datum's flat ground has reached
    /// by leaving the curving Earth.
    pub fn local_height(&self, datum: &Datum, x: f64, z: f64) -> f32 {
        let flat = datum.geodetic(DVec3::new(x, 0.0, z));
        let ground = Geodetic::new(flat.latitude, flat.longitude, 0.0).ecef();
        (self.height(ground) as f64 - flat.altitude) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIELD: (f64, f64) = (52.370216, 4.895168);

    #[test]
    fn the_home_field_is_level_where_it_starts() {
        let home = Datum::at(FIELD.0, FIELD.1);
        let land = Terrain::new(&home);
        assert!(land.local_height(&home, 0.0, 0.0).abs() < 1e-3);
        assert!(land.local_height(&home, 12.0, -9.0).abs() < 1e-3);
    }

    #[test]
    fn the_ground_falls_away_with_the_curve() {
        let home = Datum::at(FIELD.0, FIELD.1);
        let land = Terrain::new(&home);
        let drop = 20_000.0f64.powi(2) / (2.0 * crate::PLANET_RADIUS_M);
        let height = land.local_height(&home, 20_000.0, 0.0) as f64;
        assert!((height + drop).abs() < RELIEF_M as f64, "{height} vs {drop}");
    }

    #[test]
    fn two_datums_agree_on_the_land_they_share() {
        let home = Datum::at(FIELD.0, FIELD.1);
        let land = Terrain::new(&home);
        let near = home.travelled(40.0, 9_000.0);
        let spot = near.geodetic(DVec3::new(150.0, 0.0, -80.0));
        let from_home = home.local(Geodetic::new(spot.latitude, spot.longitude, 0.0));
        let a = land.local_height(&near, 150.0, -80.0) as f64 + spot.altitude;
        let seen = home.geodetic(DVec3::new(from_home.x, 0.0, from_home.z));
        let b = land.local_height(&home, from_home.x, from_home.z) as f64 + seen.altitude;
        assert!((a - b).abs() < 0.05, "{a} vs {b}");
    }
}
