//! What grows where nobody decided.
//!
//! A land is not laid out; it follows the ground's own damp. This is one
//! reading of it, coarse and slow, that the shaders can make for themselves:
//! the same noise, written the same way twice, so a mote in the air and a
//! blade in the ground always agree on where they are.

use bevy::prelude::*;

use crate::biome::Land;

/// Metres across the broad damp and dry country.
pub const BROAD_M: f32 = 420.0;
/// And of the patches within it.
pub const FINE_M: f32 = 95.0;

fn gradient(cell: Vec2) -> Vec2 {
    let hash = (cell.dot(Vec2::new(127.1, 311.7)).sin() * 43758.5453123).fract();
    let angle = hash * std::f32::consts::TAU;
    Vec2::new(angle.cos(), angle.sin())
}

/// Perlin noise in 0..1, the same as the shaders make.
pub fn noise(place: Vec2) -> f32 {
    let corner = place.floor();
    let inside = place - corner;
    let ease = inside * inside * inside * (inside * (inside * 6.0 - 15.0) + 10.0);
    let a = gradient(corner).dot(inside);
    let b = gradient(corner + Vec2::X).dot(inside - Vec2::X);
    let c = gradient(corner + Vec2::Y).dot(inside - Vec2::Y);
    let d = gradient(corner + Vec2::ONE).dot(inside - Vec2::ONE);
    let top = a + (b - a) * ease.x;
    let bottom = c + (d - c) * ease.x;
    0.5 + 0.7 * (top + (bottom - top) * ease.y)
}

/// How damp the ground is, 0 parched and 1 sodden.
pub fn damp(place: Vec2) -> f32 {
    let broad = noise(place / BROAD_M + Vec2::new(13.7, -4.1));
    let fine = noise(place / FINE_M + Vec2::new(-27.3, 8.9));
    (broad * 0.72 + fine * 0.28).clamp(0.0, 1.0)
}

/// The land somewhere, where nobody has laid one out: the damp ground grows
/// meadow, the parched grows steppe, and between them they interleave.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climate {
    /// Below this it is all steppe, above it all meadow.
    pub parched: f32,
    pub lush: f32,
    pub dry_land: Land,
    pub damp_land: Land,
}

impl Default for Climate {
    fn default() -> Self {
        Self { parched: 0.42, lush: 0.56, dry_land: Land::Steppe, damp_land: Land::Grassland }
    }
}

impl Climate {
    /// The share of the damp land at a place; the rest is the dry one.
    pub fn share(&self, place: Vec2) -> f32 {
        let t = ((damp(place) - self.parched) / (self.lush - self.parched)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_damp_stays_in_its_range_and_changes_slowly() {
        let mut least: f32 = 1.0;
        let mut most: f32 = 0.0;
        for step in 0..4000 {
            let place = Vec2::new(step as f32 * 3.1 - 6000.0, step as f32 * -1.7);
            let value = damp(place);
            assert!((0.0..=1.0).contains(&value));
            least = least.min(value);
            most = most.max(value);
            assert!((value - damp(place + Vec2::X)).abs() < 0.02);
        }
        assert!(most - least > 0.25, "the country is all one dampness: {least}..{most}");
    }

    #[test]
    fn both_lands_get_a_share_of_the_world() {
        let climate = Climate::default();
        let mut damp_country = 0;
        let mut dry_country = 0;
        for x in -60..60 {
            for z in -60..60 {
                let share = climate.share(Vec2::new(x as f32 * 40.0, z as f32 * 40.0));
                if share > 0.85 {
                    damp_country += 1;
                } else if share < 0.15 {
                    dry_country += 1;
                }
            }
        }
        assert!(damp_country > 500 && dry_country > 500, "{damp_country} damp, {dry_country} dry");
    }

    #[test]
    fn a_border_is_a_band_and_not_a_line() {
        let climate = Climate::default();
        let mut crossings = 0;
        let mut abrupt = 0;
        let mut previous = climate.share(Vec2::new(-3000.0, 21.0));
        for step in -2999..3000 {
            let share = climate.share(Vec2::new(step as f32, 21.0));
            if (share - 0.5).signum() != (previous - 0.5).signum() {
                crossings += 1;
            }
            if (share - previous).abs() > 0.25 {
                abrupt += 1;
            }
            previous = share;
        }
        assert!(crossings > 2, "only {crossings} borders in six kilometres");
        assert_eq!(abrupt, 0, "the lands meet in a line somewhere");
    }
}
