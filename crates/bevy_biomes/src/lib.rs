//! Biomes for Bevy.
//!
//! [`Biomes`] says what covers the ground anywhere in the world and how one
//! cover gives way to the next; [`Covers`] says what each is made of, layer
//! by layer. What drifts in the air over them is drawn from here too: the
//! dust, pollen and chaff that keep a view from looking swept.

mod biome;
mod blend;
mod layer;
mod motes;

use bevy::prelude::*;

pub use biome::{Biome, Border, Field, Land};
pub use blend::{Biomes, MOST, Mix, Region};
pub use layer::{Cover, Covers, MoteKind, Motes, Plants};
pub use motes::{AmbientWind, MoteBudget};

/// Everything a biome is: where it is, what it is made of, and its air.
pub struct BiomesPlugin;

impl Plugin for BiomesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Covers>()
            .init_resource::<AmbientWind>()
            .init_resource::<MoteBudget>()
            .add_plugins(motes::MotesPlugin);
    }
}
