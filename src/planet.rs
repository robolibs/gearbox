//! Planet-scale world state.
//!
//! The simulator runs on a flat tangent patch (rapier handles flat
//! physics with constant -Y gravity). A `Planet` record tracks the
//! sphere's radius and the lat/lon datum at which our local origin sits,
//! so every local XYZ position can be expressed as a geographic `Geo`
//! (WGS84-like, though we use a spherical approximation).
//!
//! For vehicles that roam within ~10 km of the datum, the ENU
//! tangent-plane projection below is accurate to << 0.1 %.

use datapod::{Geo, Point};

/// Mean Earth radius in metres.
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;

#[derive(Debug, Clone, Copy)]
pub struct Planet {
    /// Spherical body radius in metres.
    pub radius: f64,
    /// Geographic position of local (0, 0, 0).
    pub datum: Geo,
}

impl Default for Planet {
    fn default() -> Self {
        Self::earth_at(52.370216, 4.895168, 0.0) // Amsterdam, NL
    }
}

impl Planet {
    pub fn new(radius: f64, datum: Geo) -> Self {
        Self { radius, datum }
    }

    /// Earth-radius body with the supplied datum.
    pub fn earth_at(lat_deg: f64, lon_deg: f64, alt_m: f64) -> Self {
        Self {
            radius: EARTH_RADIUS_M,
            datum: Geo::new(lat_deg, lon_deg, alt_m),
        }
    }

    /// Project a local point (x East, y Up, z North, all in metres
    /// relative to the datum) to a geographic `Geo`. Uses an ENU
    /// tangent-plane spherical approximation.
    pub fn local_to_geo(&self, p: Point) -> Geo {
        let deg_per_rad = 180.0 / std::f64::consts::PI;
        let cos_lat = self.datum.latitude.to_radians().cos();
        // Guard against the ±90° poles where cos_lat → 0.
        let safe_cos_lat = cos_lat.abs().max(1e-9).copysign(cos_lat);
        Geo::new(
            self.datum.latitude + (p.z / self.radius) * deg_per_rad,
            self.datum.longitude + (p.x / (self.radius * safe_cos_lat)) * deg_per_rad,
            self.datum.altitude + p.y,
        )
    }

    /// Convert a geographic position back to local ENU coordinates.
    pub fn geo_to_local(&self, g: Geo) -> Point {
        let rad_per_deg = std::f64::consts::PI / 180.0;
        let cos_lat = self.datum.latitude.to_radians().cos();
        let safe_cos_lat = cos_lat.abs().max(1e-9).copysign(cos_lat);
        Point::new(
            (g.longitude - self.datum.longitude) * rad_per_deg * self.radius * safe_cos_lat,
            g.altitude - self.datum.altitude,
            (g.latitude - self.datum.latitude) * rad_per_deg * self.radius,
        )
    }
}
