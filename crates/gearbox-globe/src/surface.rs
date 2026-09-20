//! What the planet is made of where no field has been laid: sea, land, and the
//! ice at the poles.
//!
//! [`Terrain`] says how high the land stands, but only ever in hills — its
//! finest noise repeats every couple of kilometres, and from orbit a couple of
//! kilometres is nothing, so a planet drawn from it alone is one flat colour
//! however much relief it has. Continents are a separate, far broader layer,
//! and this is it: one land mask at continental wavelength, and the colour that
//! follows from it. It is what the globe is painted with from high up, and what
//! the distant land fades into on the way down.

use bevy::math::{DVec3, Vec3};

use crate::{Terrain, noise};

/// How wide a continent is, roughly.
const CONTINENT_M: f64 = 3_000_000.0;
/// How much of the surface stands above the water.
const LAND_SHARE: f32 = 0.42;
/// A site is put on land, and the sea is held this far off it, so wherever a
/// field is laid it has a country around it rather than an ocean. No farther:
/// held off for hundreds of kilometres, the guarantee flattens everything in
/// sight into the same inland green, which is the very thing it is here to
/// avoid. A coast should be reachable by eye from up high.
const SHORE_KEEP_M: f64 = 70_000.0;

const DEEP: Vec3 = Vec3::new(0.007, 0.024, 0.058);
const SHELF: Vec3 = Vec3::new(0.021, 0.071, 0.104);
const SAND: Vec3 = Vec3::new(0.290, 0.250, 0.155);
const GRASS: Vec3 = Vec3::new(0.042, 0.112, 0.021);
const SCRUB: Vec3 = Vec3::new(0.285, 0.225, 0.095);
const SNOW: Vec3 = Vec3::new(0.620, 0.660, 0.700);

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Sine of the latitude of an ECEF place: 0 at the equator, ±1 at the poles.
fn polar(ground: DVec3) -> f32 {
    (ground.z / ground.length().max(1.0)) as f32
}

impl Terrain {
    /// Where the coast is: 0 on it, up to 1 well inland, down to -1 in deep
    /// water. The site's own country is never under it.
    pub fn shore(&self, ground: DVec3) -> f32 {
        let broad = noise::fbm(ground / CONTINENT_M, 5);
        let bays = noise::fbm(ground / (CONTINENT_M * 0.16) + DVec3::splat(29.0), 4) - 0.5;
        let level = ((broad + bays * 0.18 - (1.0 - LAND_SHARE)) * 3.2).clamp(-1.0, 1.0);
        let out = ((ground - self.home).length() / SHORE_KEEP_M) as f32;
        let near = 1.0 - smooth(out * out);
        level + near * (1.0 - level)
    }

    /// How the land is broken up at the scale of a parish, 0 to 1: field,
    /// wood, moor and water meadow. No land is one colour, and from the air
    /// that patchwork is most of what tells you you are looking at country
    /// rather than at a painted sheet — more, at a distance, than its hills.
    pub fn patchwork(&self, ground: DVec3) -> f32 {
        noise::fbm(ground / 2_400.0 + DVec3::splat(61.0), 3)
    }

    /// The colour of the planet's surface there, linear rgb. Sea, shore, the
    /// dry and green belts of the land, and ice where the latitude is high.
    pub fn surface(&self, ground: DVec3) -> Vec3 {
        let shore = self.shore(ground);
        let water = mix(DEEP, SHELF, smooth(1.0 + shore * 3.0));
        // Belts of dry and green country, at a couple of hundred kilometres —
        // near enough to tell one from another across a view from the air,
        // where a continental wavelength is one flat colour to the horizon.
        let dry = smooth((noise::fbm(ground / (CONTINENT_M * 0.08) + DVec3::splat(7.0), 4) - 0.46) * 5.0);
        let inland = mix(SAND, mix(GRASS, SCRUB, dry), smooth(shore * 5.0));
        let ice = smooth((polar(ground).abs() - 0.80) / 0.13);
        mix(mix(water, inland, smooth(shore * 60.0)), SNOW, ice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Datum, Geodetic};

    const FIELD: (f64, f64) = (52.370216, 4.895168);

    fn home() -> Terrain {
        Terrain::new(&Datum::at(FIELD.0, FIELD.1))
    }

    fn all_over(mut visit: impl FnMut(DVec3)) {
        for lat in -8..=8 {
            for lon in 0..24 {
                let place = Geodetic::new(lat as f64 * 10.0, lon as f64 * 15.0, 0.0);
                visit(place.ecef());
            }
        }
    }

    #[test]
    fn the_polar_axis_is_ecef_z() {
        assert!(polar(Geodetic::new(90.0, 0.0, 0.0).ecef()) > 0.999);
        assert!(polar(Geodetic::new(0.0, 33.0, 0.0).ecef()).abs() < 1e-6);
    }

    #[test]
    fn the_globe_has_both_sea_and_land_on_it() {
        let land = home();
        let (mut wet, mut dry) = (0, 0);
        all_over(|p| {
            if land.shore(p) > 0.0 {
                dry += 1;
            } else {
                wet += 1;
            }
        });
        assert!(wet * 4 > dry && dry * 4 > wet, "{dry} land against {wet} sea");
    }

    #[test]
    fn the_sea_is_blue_and_the_land_is_not() {
        let land = home();
        all_over(|p| {
            let colour = land.surface(p);
            if land.shore(p) < -0.2 && polar(p).abs() < 0.7 {
                assert!(colour.z > colour.y && colour.y > colour.x, "{colour} is not water");
            }
        });
    }

    #[test]
    fn a_field_always_has_a_country_around_it() {
        for (lat, lon) in [FIELD, (-33.868, 151.209), (0.0, -160.0), (71.0, 25.0)] {
            let datum = Datum::at(lat, lon);
            let land = Terrain::new(&datum);
            assert!(land.shore(datum.origin) > 0.99, "{lat},{lon} is at sea");
            // A field's own country, out past anything a machine will drive.
            for bearing in [0.0, 90.0, 200.0, 310.0] {
                let out = datum.travelled(bearing, 25_000.0).origin;
                assert!(land.shore(out) > 0.8, "the sea comes too close to {lat},{lon}");
            }
        }
    }

    #[test]
    fn the_poles_are_white() {
        let land = home();
        for lat in [-90.0, -86.0, 86.0, 90.0] {
            let colour = land.surface(Geodetic::new(lat, 12.0, 0.0).ecef());
            assert!(colour.min_element() > 0.4, "{lat} is not iced: {colour}");
        }
    }

    #[test]
    fn the_colour_never_leaves_the_range() {
        let land = home();
        all_over(|p| {
            let colour = land.surface(p);
            assert!(colour.min_element() >= 0.0 && colour.max_element() <= 1.0, "{colour}");
        });
    }
}
