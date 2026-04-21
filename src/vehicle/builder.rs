//! Small builder API for hand-constructing a [`VehicleSpec`].
//!
//! Presets in [`crate::presets`] use this internally; external callers who
//! want to define a vehicle inline can use it too. A future YAML/URDF loader
//! will lower into the same spec.

use super::{ChassisSpec, PartSpec, VehicleSpec, WheelSpec};

pub struct VehicleBuilder {
    spec: VehicleSpec,
}

impl VehicleBuilder {
    pub fn new(name: impl Into<String>, chassis: ChassisSpec) -> Self {
        Self {
            spec: VehicleSpec {
                name: name.into(),
                chassis,
                wheels: Vec::new(),
                parts: Vec::new(),
            },
        }
    }

    pub fn wheel(mut self, wheel: WheelSpec) -> Self {
        self.spec.wheels.push(wheel);
        self
    }

    pub fn part(mut self, part: PartSpec) -> Self {
        self.spec.parts.push(part);
        self
    }

    pub fn build(self) -> VehicleSpec {
        self.spec
    }
}
