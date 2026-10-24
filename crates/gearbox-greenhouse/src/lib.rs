//! A plant growth chamber on another world, as numbers a simulator steps.
//!
//! Crops grow by NASA's Modified Energy Cascade ([`species`], [`plant`]) and
//! exchange carbon dioxide, oxygen and water with the air of the room they
//! stand in ([`air`]). Lamps light them and draw power ([`lamp`], [`power`]);
//! their roots drink from a nutrient tank ([`water`]). A [`house`] ties rooms,
//! trays, lamps, tanks and the power bus together, steps them on world time,
//! and takes the faults and repairs a crew or a robot deals in.

pub mod air;
pub mod house;
pub mod lamp;
pub mod plant;
pub mod power;
pub mod species;
pub mod water;

pub use air::{Air, Climate, Outside};
pub use house::{Event, Fault, House, PlantId, Room, Tray};
pub use lamp::{Channels, Lamp};
pub use plant::{Exchange, Plant, Stage, Surroundings};
pub use power::Power;
pub use species::{Edible, Form, Species};
pub use water::{Medium, Tank};
