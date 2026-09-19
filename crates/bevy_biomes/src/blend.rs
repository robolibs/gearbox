//! Where one biome gives way to another.
//!
//! Asked about a place, this answers with a mix rather than a name: the
//! biomes there and how much of each. A plant or a mote takes one of them at
//! random by weight, so a soft border is a band where the two interleave
//! blade by blade instead of a line where one stops. A hard border is the
//! line: inside a field there is nothing of what surrounds it.

use bevy::prelude::*;

use crate::biome::{Biome, Border};

/// A biome laid over a piece of the world.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub biome: Biome,
    pub bounds: Rect,
    pub border: Border,
}

impl Region {
    /// A region with the edge its biome keeps by default.
    pub fn new(biome: impl Into<Biome>, bounds: Rect) -> Self {
        let biome = biome.into();
        Self { biome, bounds, border: biome.border() }
    }

    pub fn with_border(mut self, border: Border) -> Self {
        self.border = border;
        self
    }
}

/// The biomes of the world: what lies everywhere, and what is laid over it.
#[derive(Resource, Clone, Debug)]
pub struct Biomes {
    /// What covers the ground where nothing else does.
    pub background: Biome,
    /// Where the background is not one land but two, by how damp the ground is.
    pub climate: Option<crate::climate::Climate>,
    pub regions: Vec<Region>,
}

impl Biomes {
    pub fn new(background: impl Into<Biome>) -> Self {
        Self { background: background.into(), climate: None, regions: Vec::new() }
    }

    /// Let the damp of the ground decide the land, instead of one land everywhere.
    pub fn under(mut self, climate: crate::climate::Climate) -> Self {
        self.climate = Some(climate);
        self
    }

    pub fn with(mut self, region: Region) -> Self {
        self.regions.push(region);
        self
    }

    /// The mix at a place.
    pub fn at(&self, place: Vec2) -> Mix {
        // A hard region owns its inside outright, the last laid winning.
        if let Some(region) = self
            .regions
            .iter()
            .rev()
            .find(|region| region.border == Border::Hard && region.bounds.contains(place))
        {
            return Mix::of(region.biome);
        }
        let mut mix = Mix::default();
        let mut taken = 0.0;
        for region in &self.regions {
            let Border::Soft { metres } = region.border else {
                continue;
            };
            let half = (metres * 0.5).max(1.0e-4);
            let share = 1.0 - smoothstep(-half, half, distance_to(region.bounds, place));
            if share > 0.0 {
                taken += share;
                mix.add(region.biome, share);
            }
        }
        let left = (1.0 - taken).max(0.0);
        match self.climate {
            Some(climate) => {
                for (land, share) in climate.lands(place) {
                    mix.add(land.into(), left * share);
                }
            }
            None => mix.add(self.background, left),
        }
        mix.settle();
        mix
    }
}

/// How many biomes may show at one place; past that the faintest are dropped.
pub const MOST: usize = 4;

/// The biomes at a place and how much of each; they add up to one, the
/// strongest first.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mix {
    shares: [Option<(Biome, f32)>; MOST],
}

impl Mix {
    pub fn of(biome: Biome) -> Self {
        let mut mix = Self::default();
        mix.shares[0] = Some((biome, 1.0));
        mix
    }

    fn add(&mut self, biome: Biome, share: f32) {
        if share <= 0.0 {
            return;
        }
        for held in &mut self.shares {
            match held {
                Some((held_biome, held_share)) if *held_biome == biome => {
                    *held_share += share;
                    return;
                }
                Some(_) => continue,
                None => {
                    *held = Some((biome, share));
                    return;
                }
            }
        }
        // Full: the newcomer takes the place of the faintest it beats.
        if let Some(weakest) = self
            .shares
            .iter_mut()
            .filter(|held| held.is_some_and(|(_, held)| held < share))
            .min_by(|a, b| a.unwrap().1.total_cmp(&b.unwrap().1))
        {
            *weakest = Some((biome, share));
        }
    }

    fn settle(&mut self) {
        self.shares.sort_by(|a, b| match (a, b) {
            (Some(a), Some(b)) => b.1.total_cmp(&a.1),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        let total: f32 = self.shares.iter().flatten().map(|(_, share)| share).sum();
        if total <= 0.0 {
            return;
        }
        for (_, share) in self.shares.iter_mut().flatten() {
            *share /= total;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (Biome, f32)> + '_ {
        self.shares.iter().flatten().copied()
    }

    pub fn strongest(&self) -> Option<Biome> {
        self.shares[0].map(|(biome, _)| biome)
    }

    pub fn share_of(&self, biome: Biome) -> f32 {
        self.iter().find(|(held, _)| *held == biome).map_or(0.0, |(_, share)| share)
    }

    /// One biome of the mix, by weight: `roll` is anything in 0..1. This is
    /// how a single plant or mote decides which biome it belongs to, and so
    /// how a soft border comes to be a band of the two mingled.
    pub fn pick(&self, roll: f32) -> Option<Biome> {
        let mut left = roll.clamp(0.0, 1.0);
        let mut last = None;
        for (biome, share) in self.iter() {
            last = Some(biome);
            if left < share {
                return Some(biome);
            }
            left -= share;
        }
        last
    }
}

fn smoothstep(low: f32, high: f32, value: f32) -> f32 {
    let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far a place is outside a rectangle; negative inside it.
fn distance_to(bounds: Rect, place: Vec2) -> f32 {
    let outside = (bounds.min - place).max(place - bounds.max);
    outside.max(Vec2::ZERO).length() + outside.max_element().min(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::{Field, Land};

    fn square(centre: Vec2, side: f32) -> Rect {
        Rect::from_center_size(centre, Vec2::splat(side))
    }

    fn world() -> Biomes {
        Biomes::new(Land::Grassland)
            .with(Region::new(Land::Steppe, square(Vec2::new(100.0, 0.0), 100.0)))
            .with(Region::new(Field::Stubble, square(Vec2::ZERO, 40.0)))
    }

    #[test]
    fn nowhere_in_particular_is_the_background() {
        let mix = world().at(Vec2::new(-400.0, -400.0));
        assert_eq!(mix.strongest(), Some(Land::Grassland.into()));
        assert!((mix.share_of(Land::Grassland.into()) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_field_owns_its_inside_and_keeps_a_crisp_edge() {
        let world = world();
        let inside = world.at(Vec2::new(19.99, 0.0));
        let outside = world.at(Vec2::new(20.01, 0.0));
        assert!((inside.share_of(Field::Stubble.into()) - 1.0).abs() < 1e-6);
        assert_eq!(outside.share_of(Field::Stubble.into()), 0.0);
        assert!((outside.share_of(Land::Grassland.into()) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn lands_interleave_across_their_border() {
        let world = world();
        let edge = world.at(Vec2::new(50.0, 0.0));
        assert!((edge.share_of(Land::Steppe.into()) - 0.5).abs() < 0.02, "{edge:?}");
        assert!((edge.share_of(Land::Grassland.into()) - 0.5).abs() < 0.02);
        let just_inside = world.at(Vec2::new(56.0, 0.0));
        let just_outside = world.at(Vec2::new(44.0, 0.0));
        assert!(just_inside.share_of(Land::Steppe.into()) > 0.98);
        assert!(just_outside.share_of(Land::Steppe.into()) < 0.02);
    }

    #[test]
    fn a_mix_always_adds_up_to_one() {
        let world = world();
        for x in -200..200 {
            let mix = world.at(Vec2::new(x as f32 * 1.7, 0.0));
            let total: f32 = mix.iter().map(|(_, share)| share).sum();
            assert!((total - 1.0).abs() < 1e-5, "{x} {mix:?}");
        }
    }

    #[test]
    fn picking_follows_the_shares() {
        let mix = world().at(Vec2::new(50.0, 0.0));
        let rolls = 2000;
        let steppe = (0..rolls)
            .filter(|roll| {
                mix.pick(*roll as f32 / rolls as f32) == Some(Land::Steppe.into())
            })
            .count();
        assert!((steppe as f32 / rolls as f32 - 0.5).abs() < 0.03);
    }

    #[test]
    fn the_strongest_comes_first() {
        let mix = world().at(Vec2::new(53.0, 0.0));
        let shares: Vec<f32> = mix.iter().map(|(_, share)| share).collect();
        assert!(shares.windows(2).all(|pair| pair[0] >= pair[1]), "{shares:?}");
        assert_eq!(mix.strongest(), Some(Land::Steppe.into()));
    }
}

#[cfg(test)]
mod climate_tests {
    use super::*;
    use crate::biome::{Field, Land};
    use crate::climate::Climate;

    #[test]
    fn the_background_follows_the_damp_when_it_is_asked_to() {
        let world = Biomes::new(Land::Grassland).under(Climate::default());
        let (mut meadow, mut steppe, mut sand) = (0, 0, 0);
        for step in 0..900 {
            let place = Vec2::new(step as f32 * 31.0 - 14000.0, step as f32 * 17.0);
            let mix = world.at(place);
            let total: f32 = mix.iter().map(|(_, share)| share).sum();
            assert!((total - 1.0).abs() < 1e-5);
            match mix.strongest() {
                Some(Biome::Land(Land::Grassland)) => meadow += 1,
                Some(Biome::Land(Land::Steppe)) => steppe += 1,
                Some(Biome::Land(Land::Desert)) => sand += 1,
                other => panic!("nothing grows there: {other:?}"),
            }
        }
        assert!(meadow > 60 && steppe > 60 && sand > 20, "{meadow} meadow, {steppe} steppe, {sand} sand");
    }

    #[test]
    fn a_field_still_owns_its_inside_whatever_the_weather() {
        let world = Biomes::new(Land::Grassland)
            .under(Climate::default())
            .with(Region::new(Field::Stubble, Rect::from_center_size(Vec2::ZERO, Vec2::splat(40.0))));
        assert!((world.at(Vec2::new(10.0, -10.0)).share_of(Field::Stubble.into()) - 1.0).abs() < 1e-6);
    }
}
