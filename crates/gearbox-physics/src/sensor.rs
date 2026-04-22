//! Sensor abstraction — placeholder for phase 2.
//!
//! The goal is a trait-based system: each sensor reads from the rapier world
//! plus a vehicle handle and produces a reading. Nothing implements this yet;
//! the trait exists so downstream code can start depending on it.

use gearbox_core::VehicleId;

use crate::Sim;

pub trait Sensor {
    type Reading;

    /// Attach / reset the sensor against a live vehicle.
    fn attach(&mut self, sim: &Sim, vehicle: VehicleId);

    /// Produce a reading from the current world state.
    fn sample(&mut self, sim: &Sim) -> Self::Reading;
}
