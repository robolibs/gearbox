//! The biomes, by name.
//!
//! A biome is either a [`Land`], which is what grows somewhere when nobody
//! tends it, or a [`Field`], which is a parcel somebody works. The difference
//! is not only what stands in them: lands run into one another over tens of
//! metres, while a field has the edge a plough left. One land holds many
//! fields, and the fields in it need not resemble it or each other.

use bevy::prelude::*;

/// What grows somewhere of its own accord. Lands meet gradually.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Reflect)]
pub enum Land {
    Grassland,
    Steppe,
    Savanna,
    Tundra,
    Taiga,
    TemperateForest,
    TropicalForest,
    Chaparral,
    MediterraneanShrubland,
    XericShrubland,
    Wetland,
    Mangrove,
    Desert,
    ColdDesert,
    Alpine,
    Volcanic,
    IceCap,
    Barren,
    Ocean,
}

/// A parcel somebody works, or has paved. Fields keep a crisp edge.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Reflect)]
pub enum Field {
    Meadow,
    Pasture,
    Wheat,
    Stubble,
    Maize,
    Rapeseed,
    Potato,
    Beet,
    Ploughed,
    Seeded,
    Orchard,
    Vineyard,
    Yard,
    Gravel,
    Track,
    Asphalt,
    Pond,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Reflect)]
pub enum Biome {
    Land(Land),
    Field(Field),
}

/// How a biome meets whatever is next to it.
#[derive(Clone, Copy, PartialEq, Debug, Reflect)]
pub enum Border {
    /// The edge a plough or a kerb left: one biome stops and the next starts.
    Hard,
    /// The two interleave over this many metres, each thinning into the other.
    Soft { metres: f32 },
}

impl Biome {
    /// What kind of edge this biome keeps unless it is told otherwise.
    pub fn border(self) -> Border {
        match self {
            Self::Land(_) => Border::Soft { metres: 12.0 },
            Self::Field(_) => Border::Hard,
        }
    }

    pub fn land(self) -> Option<Land> {
        match self {
            Self::Land(land) => Some(land),
            Self::Field(_) => None,
        }
    }

    pub fn field(self) -> Option<Field> {
        match self {
            Self::Field(field) => Some(field),
            Self::Land(_) => None,
        }
    }
}

impl From<Land> for Biome {
    fn from(land: Land) -> Self {
        Self::Land(land)
    }
}

impl From<Field> for Biome {
    fn from(field: Field) -> Self {
        Self::Field(field)
    }
}
