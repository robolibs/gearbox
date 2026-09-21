//! What grows where nobody decided.
//!
//! A land is not laid out; it follows the ground's own damp. This is one
//! reading of it, coarse and slow, that the shaders can make for themselves:
//! the same noise, written the same way twice, so a mote in the air and a
//! blade in the ground always agree on where they are.

use bevy::prelude::*;

use crate::biome::Land;

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
///
/// Noise on a lattice shows its lattice: diamonds, and edges along the axes.
/// Folding the place through a coarse reading of itself, and turning each
/// finer reading against the last, leaves nothing straight to see. The
/// shaders read it the same way, term for term.
pub fn damp(place: Vec2) -> f32 {
    let turn = Mat2::from_cols(Vec2::new(0.80, 0.60), Vec2::new(-0.60, 0.80));
    let warp = Vec2::new(
        noise(place / 610.0 + Vec2::new(5.2, 1.3)),
        noise(place / 610.0 + Vec2::new(-3.1, 7.7)),
    ) - Vec2::splat(0.5);
    let folded = place + warp * 260.0;
    let broad = noise(folded / 430.0 + Vec2::new(13.7, -4.1));
    let middle = noise(turn * folded / 170.0 + Vec2::new(-27.3, 8.9));
    let fine = noise(turn * turn * folded / 68.0 + Vec2::new(41.0, -9.3));
    (broad * 0.54 + middle * 0.31 + fine * 0.15).clamp(0.0, 1.0)
}

/// The lands where nobody has laid one out, in the order the ground's damp
/// puts them: sand where it is parched, dry grass over it, meadow where the
/// water is. Each gives way to the next across a band, never at a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climate {
    /// The damp at which the sand has wholly given way, and where it begins to.
    pub sand: (f32, f32),
    /// And the damp between which dry grass becomes meadow.
    pub lush: (f32, f32),
    pub parched_land: Land,
    pub dry_land: Land,
    pub damp_land: Land,
}

impl Default for Climate {
    fn default() -> Self {
        Self {
            // A fifth of the country is sand, a third meadow, the rest dry grass:
            // the readings are where the damp falls, not round numbers.
            sand: (0.395, 0.445),
            lush: (0.505, 0.570),
            parched_land: Land::Desert,
            dry_land: Land::Steppe,
            damp_land: Land::Grassland,
        }
    }
}

fn ease(low: f32, high: f32, value: f32) -> f32 {
    let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Climate {
    /// How much of each land there is at a place: parched, dry, damp. They
    /// add up to one.
    pub fn shares(&self, place: Vec2) -> [f32; 3] {
        let wet = damp(place);
        let parched = 1.0 - ease(self.sand.0, self.sand.1, wet);
        let damp_share = ease(self.lush.0, self.lush.1, wet);
        [parched, (1.0 - parched - damp_share).max(0.0), damp_share]
    }

    /// The lands at a place with their shares.
    pub fn lands(&self, place: Vec2) -> [(Land, f32); 3] {
        let shares = self.shares(place);
        [
            (self.parched_land, shares[0]),
            (self.dry_land, shares[1]),
            (self.damp_land, shares[2]),
        ]
    }

    /// The share of the damp land at a place, for anything that only wants to
    /// know how green it is.
    pub fn share(&self, place: Vec2) -> f32 {
        self.shares(place)[2]
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
            assert!((value - damp(place + Vec2::X)).abs() < 0.03);
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

#[cfg(test)]
mod land_tests {
    use super::*;

    #[test]
    fn the_three_lands_always_add_up_to_one() {
        let climate = Climate::default();
        for step in 0..3000 {
            let place = Vec2::new(step as f32 * 23.0 - 30000.0, step as f32 * -11.0);
            let shares = climate.shares(place);
            assert!(shares.iter().all(|share| (0.0..=1.0).contains(share)), "{shares:?}");
            assert!((shares.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{shares:?}");
        }
    }

    #[test]
    fn there_is_sand_dry_grass_and_meadow_in_the_world() {
        let climate = Climate::default();
        let mut found = [0; 3];
        for x in -70..70 {
            for z in -70..70 {
                let shares = climate.shares(Vec2::new(x as f32 * 55.0, z as f32 * 55.0));
                for (count, share) in found.iter_mut().zip(shares) {
                    if share > 0.8 {
                        *count += 1;
                    }
                }
            }
        }
        assert!(found.iter().all(|count| *count > 120), "{found:?}");
    }

    #[test]
    fn sand_never_touches_meadow_without_dry_grass_between() {
        let climate = Climate::default();
        for step in 0..6000 {
            let shares = climate.shares(Vec2::new(step as f32 * 1.7, 42.0));
            assert!(shares[0] * shares[2] < 0.06, "{shares:?}");
        }
    }
}

#[cfg(test)]
mod spread {
    use super::*;

    #[test]
    #[ignore]
    fn print_the_spread_of_the_damp() {
        let mut readings: Vec<f32> = (0..200_000)
            .map(|step| {
                let place = Vec2::new((step % 450) as f32 * 37.0, (step / 450) as f32 * 41.0);
                damp(place)
            })
            .collect();
        readings.sort_by(f32::total_cmp);
        let at = |q: f32| readings[(q * (readings.len() - 1) as f32) as usize];
        println!(
            "5% {:.3}  20% {:.3}  35% {:.3}  50% {:.3}  65% {:.3}  80% {:.3}  95% {:.3}",
            at(0.05), at(0.20), at(0.35), at(0.50), at(0.65), at(0.80), at(0.95)
        );
    }
}
