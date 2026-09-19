//! What a biome is made of.
//!
//! A cover is read from the ground up: the soil, the plants standing in it,
//! and what drifts in the air above them. Each layer says how much of itself
//! there is and how far out it is worth drawing; together they are the whole
//! description of a place, and two covers are mixed by mixing their layers.

use bevy::prelude::*;

use crate::biome::{Biome, Field, Land};

/// Plants standing in the ground. The blades themselves are drawn elsewhere;
/// this is the recipe a renderer is handed.
#[derive(Clone, Copy, Debug, Reflect)]
pub struct Plants {
    pub name: &'static str,
    pub per_square_metre: f32,
    /// Full density up to the first, gone by the second.
    pub reach_m: [f32; 2],
    pub height_m: [f32; 2],
}

/// What drifts in the air: the grain of the view that keeps it from looking
/// swept. Rising motes recycle at the ceiling, settling ones at the ground.
#[derive(Clone, Copy, Debug, Reflect)]
pub struct Motes {
    pub kind: MoteKind,
    pub per_hectare: f32,
    pub size_mm: [f32; 2],
    /// Metres a second: positive rises, negative settles.
    pub rise_mps: f32,
    /// How much of the wind it takes: 1 goes with it, 0 hangs still.
    pub drag: f32,
    /// How high above the ground they reach.
    pub ceiling_m: f32,
    /// Beyond this they are too small to make out.
    pub reach_m: f32,
    pub colour: Srgba,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Reflect)]
pub enum MoteKind {
    /// Soil lifted off dry ground: a round, dim speck.
    Dust,
    /// Fine and bright, and it hangs where it is.
    Pollen,
    /// Broad and slow, turning over as it falls.
    Petal,
    /// A long splinter of stalk, end over end.
    Chaff,
    /// A tuft that hardly falls at all.
    Seed,
}

/// Everything about one biome.
#[derive(Clone, Debug)]
pub struct Cover {
    pub biome: Biome,
    pub plants: Vec<Plants>,
    pub air: Vec<Motes>,
}

impl Cover {
    pub fn new(biome: impl Into<Biome>) -> Self {
        Self { biome: biome.into(), plants: Vec::new(), air: Vec::new() }
    }

    pub fn with_plants(mut self, plants: Plants) -> Self {
        self.plants.push(plants);
        self
    }

    pub fn with_air(mut self, motes: Motes) -> Self {
        self.air.push(motes);
        self
    }
}

/// Every cover the world knows, by biome.
#[derive(Resource, Default)]
pub struct Covers(Vec<Cover>);

impl Covers {
    pub fn insert(&mut self, cover: Cover) {
        self.0.retain(|held| held.biome != cover.biome);
        self.0.push(cover);
    }

    pub fn get(&self, biome: Biome) -> Option<&Cover> {
        self.0.iter().find(|cover| cover.biome == biome)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Cover> {
        self.0.iter()
    }
}

impl Covers {
    /// The covers of the biomes the world has so far. Everything else is a
    /// name waiting for a recipe.
    pub fn built_in() -> Self {
        let mut covers = Self::default();
        covers.insert(
            Cover::new(Land::Grassland)
                .with_air(Motes {
                    kind: MoteKind::Pollen,
                    per_hectare: 2600.0,
                    size_mm: [4.0, 9.0],
                    rise_mps: 0.02,
                    drag: 0.55,
                    ceiling_m: 2.4,
                    reach_m: 22.0,
                    colour: Srgba::new(0.9, 0.88, 0.76, 0.06),
                })
                .with_air(Motes {
                    kind: MoteKind::Petal,
                    per_hectare: 380.0,
                    size_mm: [14.0, 26.0],
                    rise_mps: -0.22,
                    drag: 0.85,
                    ceiling_m: 2.2,
                    reach_m: 30.0,
                    colour: Srgba::new(0.9, 0.84, 0.83, 0.16),
                })
                .with_air(Motes {
                    kind: MoteKind::Seed,
                    per_hectare: 220.0,
                    size_mm: [10.0, 20.0],
                    rise_mps: 0.06,
                    drag: 0.95,
                    ceiling_m: 2.8,
                    reach_m: 24.0,
                    colour: Srgba::new(0.88, 0.88, 0.83, 0.07),
                }),
        );
        covers.insert(
            Cover::new(Field::Stubble)
                .with_air(Motes {
                    kind: MoteKind::Chaff,
                    per_hectare: 420.0,
                    size_mm: [16.0, 42.0],
                    rise_mps: -0.12,
                    drag: 0.95,
                    ceiling_m: 1.9,
                    reach_m: 26.0,
                    colour: Srgba::new(0.78, 0.7, 0.52, 0.17),
                })
                .with_air(Motes {
                    kind: MoteKind::Dust,
                    per_hectare: 2000.0,
                    size_mm: [8.0, 20.0],
                    rise_mps: 0.08,
                    drag: 0.8,
                    ceiling_m: 3.2,
                    reach_m: 28.0,
                    colour: Srgba::new(0.72, 0.66, 0.55, 0.05),
                }),
        );
        covers.insert(Cover::new(Field::Yard).with_air(Motes {
            kind: MoteKind::Dust,
            per_hectare: 600.0,
            size_mm: [6.0, 14.0],
            rise_mps: 0.04,
            drag: 0.7,
            ceiling_m: 3.0,
            reach_m: 26.0,
            colour: Srgba::new(0.7, 0.69, 0.66, 0.05),
        }));
        covers
    }
}
