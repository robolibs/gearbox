//! The Earth as the simulator sees it.
//!
//! Where anything is, is said in WGS84: latitude, longitude and altitude, or
//! the same place as ECEF metres from the Earth's centre. Both are exact and
//! global, and neither is fit to simulate in: ECEF numbers are huge and its
//! axes point nowhere useful. So whatever stands on the ground is simulated in
//! the flat frame of a [`Datum`], an anchor at some latitude and longitude with
//! x north, y up and z east. A datum is a convenience of the moment; a place's
//! latitude, longitude and altitude are what it keeps.

mod noise;
mod surface;
mod terrain;

use bevy::math::{DMat3, DQuat, DVec3};
use concord::earth::{Wgs, to_ecf, to_wgs};

pub use terrain::{RELIEF_M, Terrain};

/// Radius of the sphere drawn for the planet and its sky.
pub const PLANET_RADIUS_M: f64 = 6_371_000.0;
/// A datum serves the land this far around it; past that its flat frame
/// leans too far from the true vertical.
pub const DATUM_REACH_M: f64 = 25_000.0;
/// Datums sit this far apart along X in the one physics world they share.
pub const REGION_SPACING_M: f64 = 1.0e6;

/// Latitude and longitude in degrees, altitude in metres above the ellipsoid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geodetic {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
}

impl Geodetic {
    pub fn new(latitude: f64, longitude: f64, altitude: f64) -> Self {
        Self { latitude, longitude, altitude }
    }

    pub fn ecef(&self) -> DVec3 {
        let p = to_ecf(Wgs::new(self.latitude, self.longitude, self.altitude));
        DVec3::new(p.x, p.y, p.z)
    }

    pub fn of_ecef(ecef: DVec3) -> Self {
        let wgs = to_wgs(concord::earth::Ecf::new(ecef.x, ecef.y, ecef.z));
        Self::new(wgs.latitude, wgs.longitude, wgs.altitude)
    }
}

/// A flat frame anchored on the ellipsoid: x north, y up, z east.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Datum {
    pub latitude: f64,
    pub longitude: f64,
    /// The anchor in ECEF.
    pub origin: DVec3,
    /// Turns the datum's axes into ECEF's.
    pub rotation: DQuat,
}

impl Datum {
    pub fn at(latitude: f64, longitude: f64) -> Self {
        let (lat, lon) = (latitude.to_radians(), longitude.to_radians());
        let east = DVec3::new(-lon.sin(), lon.cos(), 0.0);
        let north = DVec3::new(-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos());
        let up = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
        Self {
            latitude,
            longitude,
            origin: Geodetic::new(latitude, longitude, 0.0).ecef(),
            rotation: DQuat::from_mat3(&DMat3::from_cols(north, up, east)),
        }
    }

    /// The datum on the ground under an ECEF place.
    pub fn under(ecef: DVec3) -> Self {
        let place = Geodetic::of_ecef(ecef);
        Self::at(place.latitude, place.longitude)
    }

    pub fn up(&self) -> DVec3 {
        self.rotation * DVec3::Y
    }

    pub fn to_ecef(&self, local: DVec3) -> DVec3 {
        self.origin + self.rotation * local
    }

    pub fn from_ecef(&self, ecef: DVec3) -> DVec3 {
        self.rotation.inverse() * (ecef - self.origin)
    }

    pub fn geodetic(&self, local: DVec3) -> Geodetic {
        Geodetic::of_ecef(self.to_ecef(local))
    }

    pub fn local(&self, place: Geodetic) -> DVec3 {
        self.from_ecef(place.ecef())
    }

    /// How far along the datum's ground an ECEF place is from the anchor.
    pub fn ground_distance(&self, ecef: DVec3) -> f64 {
        let local = self.from_ecef(ecef);
        local.x.hypot(local.z)
    }

    /// The datum `distance` metres away on compass `bearing` degrees, along a
    /// great circle of the mean sphere: for going places, not for surveying.
    pub fn travelled(&self, bearing: f64, distance: f64) -> Self {
        let (lat, lon, b) = (self.latitude.to_radians(), self.longitude.to_radians(), bearing.to_radians());
        let angle = distance / PLANET_RADIUS_M;
        let to_lat = (lat.sin() * angle.cos() + lat.cos() * angle.sin() * b.cos()).asin();
        let to_lon = lon + (b.sin() * angle.sin() * lat.cos()).atan2(angle.cos() - lat.sin() * to_lat.sin());
        Self::at(to_lat.to_degrees(), (to_lon.to_degrees() + 540.0).rem_euclid(360.0) - 180.0)
    }
}

/// The region of the shared physics world that belongs to datum `index`.
pub fn physics_offset(index: usize) -> DVec3 {
    DVec3::X * (index as f64 * REGION_SPACING_M)
}

/// The datum whose region holds physics coordinate `x`.
pub fn region_of_physics(x: f64) -> usize {
    (x / REGION_SPACING_M).round().max(0.0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIELD: (f64, f64) = (52.370216, 4.895168);

    #[test]
    fn a_datum_has_north_up_and_east_for_axes() {
        let datum = Datum::at(FIELD.0, FIELD.1);
        let north = datum.geodetic(DVec3::new(1000.0, 0.0, 0.0));
        let east = datum.geodetic(DVec3::new(0.0, 0.0, 1000.0));
        let above = datum.geodetic(DVec3::new(0.0, 50.0, 0.0));
        assert!(north.latitude > FIELD.0 && (north.longitude - FIELD.1).abs() < 1e-6);
        assert!(east.longitude > FIELD.1 && (east.latitude - FIELD.0).abs() < 1e-3);
        assert!((above.altitude - 50.0).abs() < 1e-6 && (above.latitude - FIELD.0).abs() < 1e-9);
    }

    #[test]
    fn places_round_trip_through_any_datum() {
        for (lat, lon) in [FIELD, (-33.9, 151.2), (0.0, -179.9), (89.0, 10.0)] {
            let datum = Datum::at(lat, lon);
            let local = DVec3::new(1234.5, 6.7, -890.1);
            assert!((datum.from_ecef(datum.to_ecef(local)) - local).length() < 1e-6);
            let place = datum.geodetic(local);
            assert!((datum.local(place) - local).length() < 1e-4);
        }
    }

    #[test]
    fn a_metre_north_is_a_metre() {
        let datum = Datum::at(FIELD.0, FIELD.1);
        let a = datum.geodetic(DVec3::ZERO).ecef();
        let b = datum.geodetic(DVec3::new(1.0, 0.0, 0.0)).ecef();
        assert!(((a - b).length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn travelling_half_way_round_reaches_the_far_side() {
        let home = Datum::at(FIELD.0, FIELD.1);
        let far = home.travelled(90.0, std::f64::consts::PI * PLANET_RADIUS_M * 0.999);
        assert!(far.up().dot(home.up()) < -0.99);
    }

    #[test]
    fn physics_regions_map_back_to_their_datum() {
        for index in [0usize, 1, 7, 300] {
            let inside = physics_offset(index) + DVec3::new(-30_000.0, 5.0, 40_000.0);
            assert_eq!(region_of_physics(inside.x), index);
        }
    }
}
