//! Bevy plugin layer — wraps [`ApiBroker`] and wires it to the
//! editor's `SimClock` resource. Only compiled with the `bevy`
//! feature; the headless `gearbox-server` binary disables it.

use bevy::prelude::*;

use gearbox_viz::{SimClock, SimSpeed};

use crate::broker::ApiBroker;
use crate::wire::{ClockCommand, ClockWire};

pub struct GearboxApiPlugin;

#[derive(Resource)]
pub struct ApiSession {
    pub broker: ApiBroker,
}

impl Plugin for GearboxApiPlugin {
    fn build(&self, app: &mut App) {
        match ApiBroker::open() {
            Ok(broker) => {
                info!("gearbox-api: zenoh session open");
                app.insert_resource(ApiSession { broker });
                app.add_systems(
                    PostUpdate,
                    (apply_clock_commands_system, publish_clock_system).chain(),
                );
            }
            Err(e) => {
                warn!("gearbox-api: broker open failed ({e}); API disabled");
            }
        }
    }
}

fn apply_clock_commands_system(
    api: Option<Res<ApiSession>>,
    mut clock: ResMut<SimClock>,
) {
    let Some(api) = api else { return };
    for cmd in api.broker.drain_clock_commands() {
        match cmd {
            ClockCommand::SetPaused(p) => clock.paused = p,
            ClockCommand::SetSpeed(s) => clock.speed = snap_to_sim_speed(s),
        }
    }
}

fn publish_clock_system(api: Option<Res<ApiSession>>, clock: Res<SimClock>) {
    let Some(api) = api else { return };
    api.broker.publish_clock(&ClockWire {
        paused: clock.paused,
        speed: clock.speed.multiplier(),
    });
}

/// The editor's `SimClock` is driven by a discrete enum (1×/2×/4×/8×),
/// but the wire protocol uses a continuous `f32` multiplier so
/// non-Bevy servers don't need to share the preset set. Snap the
/// requested speed to the closest preset.
fn snap_to_sim_speed(requested: f32) -> SimSpeed {
    if requested >= 6.0 {
        SimSpeed::X8
    } else if requested >= 3.0 {
        SimSpeed::X4
    } else if requested >= 1.5 {
        SimSpeed::X2
    } else {
        SimSpeed::X1
    }
}
