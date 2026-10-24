//! The land: one height above the ellipsoid for every place on Earth.

use bevy::math::DVec3;

use crate::{Body, Datum, Geodetic, noise, regolith};

const FLAT_RADIUS_M: f64 = 30.0;
const GENTLE_RADIUS_M: f64 = 70.0;
const RELIEF_FROM_M: f64 = 400.0;
const RELIEF_FULL_M: f64 = 2_000.0;
/// Peak-to-trough height of the broad relief away from the home field.
pub const RELIEF_M: f32 = 160.0;
/// Off Earth, home is a prepared pad an outpost stands on: level within this
/// radius, metres. The globe's bake shader keeps the same pad.
pub const PAD_FLAT_M: f64 = 30.0;
/// The pad's edge is back to the land's full relief this far out.
pub const PAD_RELIEF_M: f64 = 60.0;

fn smooth(t: f64) -> f32 {
    let t = t.clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

/// The land. It is level where `home` is anchored, and gentle around that, so
/// a field can be laid there; the broad relief rises farther out.
#[derive(Clone, Copy, Debug)]
pub struct Terrain {
    body: Body,
    home: DVec3,
    home_raw: f32,
}

impl Terrain {
    pub fn body(&self) -> Body { self.body }
    pub fn bake_home(&self) -> [f32; 4] {
        [self.home.x as f32, self.home.y as f32, self.home.z as f32, self.home_raw]
    }

    pub fn new(home: &Datum) -> Self {
        let raw = if home.body == Body::Earth { Self::meadow(home.origin) } else { regolith::height(home.body, home.origin) };
        Self { body: home.body, home: home.origin, home_raw: raw }
    }

    fn meadow(p: DVec3) -> f32 {
        (noise::fbm(p * 0.0045, 5) - 0.5) * 7.0
            + (noise::fbm(p * 0.035 + DVec3::splat(11.0), 3) - 0.5) * 0.5
            + (noise::fbm(p * 0.0011 + DVec3::splat(53.0), 3) - 0.5) * 34.0
    }

    /// Height above the ellipsoid of the land over `ground`, an ECEF place on it.
    pub fn height(&self, ground: DVec3) -> f32 {
        let from_home = (ground - self.home).length();
        if self.body != Body::Earth {
            let pad = smooth((from_home - PAD_FLAT_M) / (PAD_RELIEF_M - PAD_FLAT_M));
            return (regolith::height(self.body, ground) - self.home_raw) * pad;
        }
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
        if self.body != Body::Earth {
            let base = datum.to_ecef(DVec3::new(x, 0.0, z));
            let up = datum.rotation * DVec3::Y;
            let radius = self.body.radius_m();
            let mut y = self.height(base.normalize() * radius) as f64 - (base.length() - radius);
            for _ in 0..4 {
                let point = base + up * y;
                let radial = point.normalize();
                let error = point.length() - radius - self.height(radial * radius) as f64;
                if error.abs() < 0.005 { break; }
                y -= error / radial.dot(up).max(0.25);
            }
            return y as f32;
        }
        let flat = datum.geodetic(DVec3::new(x, 0.0, z));
        let ground = Geodetic::new(flat.latitude, flat.longitude, 0.0).ecef_on(self.body);
        (self.height(ground) as f64 - flat.altitude) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIELD: (f64, f64) = (52.370216, 4.895168);

    #[test]
    fn planet_tile_edges_lie_on_shared_radial_surface() {
        for body in [Body::Mars, Body::Moon] {
            let (lat, lon) = body.home();
            let home = Datum::on(body, lat, lon);
            let terrain = Terrain::new(&home);
            let mut worst = 0.0_f64;
            for site in [home, Datum::on(body, 70.0, 150.0), Datum::on(body, -45.0, -80.0)] {
                for center in [0.0, 25_000.0] {
                    for side in [-600.0, 600.0] {
                        for along in [-600.0, 0.0, 600.0] {
                            for (x, z) in [(center + side, along), (center + along, side)] {
                                let y = terrain.local_height(&site, x, z);
                                let p = site.to_ecef(DVec3::new(x, y as f64, z));
                                let radial = p.normalize() * body.radius_m();
                                let error = (p.length() - body.radius_m() - terrain.height(radial) as f64).abs();
                                worst = worst.max(error);
                            }
                        }
                    }
                }
            }
            assert!(worst < 0.1, "{body:?} tile edge radial mismatch {worst} m");
        }
    }

    #[test]
    fn planet_sites_agree_on_shared_craters_and_home_clearance() {
        for body in [Body::Mars, Body::Moon] {
            let (lat, lon) = body.home();
            let home = Datum::on(body, lat, lon);
            let land = Terrain::new(&home);
            assert!(land.local_height(&home, 0.0, 0.0).abs() < 1e-4);
            assert!(land.local_height(&home, 2.0, -2.0).abs() < 1e-4);
            // The pad holds an outpost: level to its edge, and the land rises again past it.
            let edge = PAD_FLAT_M * 0.7;
            assert!(land.local_height(&home, edge, edge).abs() < 0.02, "{body:?} pad edge");
            let beyond = (-400..=400).step_by(40).map(|x| land.local_height(&home, 300.0, x as f64).abs()).fold(0.0, f32::max);
            assert!(beyond > 0.5, "{body:?} relief returns past the pad, {beyond} m");
            for distance in [20_000.0, body.radius_m() * 2.8] {
                let remote = home.travelled(65.0, distance);
                let point = remote.geodetic(DVec3::new(150.0, 0.0, 80.0));
                let raw = land.height(Geodetic::new(point.latitude, point.longitude, 0.0).ecef_on(body));
                let local = land.local_height(&remote, 150.0, 80.0);
                assert!((local as f64 + point.altitude - raw as f64).abs() < 1e-3);
            }
        }
    }

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
