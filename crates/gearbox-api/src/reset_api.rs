//! **Pluggable** scene-reset API — wipe every vehicle and every
//! marker without restarting the simulator. Lets a python driver
//! reset the world to a clean slate before each run.
//!
//! Same delete-it-later pattern as the rest of `gearbox-api`:
//!
//!   1. delete this file,
//!   2. drop `pub mod reset_api;` + the `reset_api::*` re-exports
//!      in `lib.rs`,
//!   3. drop `app.add_plugins(ResetApiPlugin)` in `main.rs`.
//!
//! ## Topic
//!
//! | direction | key                  | payload         |
//! |-----------|----------------------|-----------------|
//! | sub       | `gearbox/sim/reset`  | [`ResetWire`]   |
//!
//! Publishing an empty payload (or any malformed one) is treated as
//! a default reset (`pause_clock = false`).

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use zenoh::Wait;

use crate::wire::decode;

#[cfg(feature = "bevy")]
use bevy::prelude::*;

#[cfg(feature = "bevy")]
use gearbox_viz::SimResetRequest;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ResetWire {
    /// Re-pause the sim after the reset. Off by default — most
    /// callers want to drive a freshly-spawned vehicle right away.
    #[serde(default)]
    pub pause_clock: bool,
}

// ─── Broker ────────────────────────────────────────────────────────

pub struct ResetBroker {
    _session: Arc<zenoh::Session>,
    /// Latched reset request (most recent wins; bursts collapse to a
    /// single reset). The Bevy system drains by `take()`-ing.
    pending: Arc<Mutex<Option<ResetWire>>>,
    _subscriber: zenoh::pubsub::Subscriber<()>,
}

impl ResetBroker {
    pub fn open(
        session: Arc<zenoh::Session>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let pending: Arc<Mutex<Option<ResetWire>>> = Arc::new(Mutex::new(None));
        let pending_cb = Arc::clone(&pending);
        let subscriber = session
            .declare_subscriber("gearbox/sim/reset")
            .callback(move |sample| {
                // Empty / malformed payloads are normalized to a
                // default reset — `python … session.put("gearbox/sim/reset", b"")`
                // should "just work" without forcing the caller to
                // CBOR-encode a struct.
                let bytes = sample.payload().to_bytes();
                let req =
                    decode::<ResetWire>(bytes.as_ref()).unwrap_or_else(|_| ResetWire::default());
                if let Ok(mut q) = pending_cb.lock() {
                    *q = Some(req);
                }
            })
            .wait()?;
        Ok(Self {
            _session: session,
            pending,
            _subscriber: subscriber,
        })
    }

    pub fn take(&self) -> Option<ResetWire> {
        self.pending.lock().ok().and_then(|mut g| g.take())
    }
}

// ─── Bevy plugin ───────────────────────────────────────────────────

#[cfg(feature = "bevy")]
#[derive(Resource)]
pub struct ResetApiSession {
    pub broker: Mutex<ResetBroker>,
}

#[cfg(feature = "bevy")]
pub struct ResetApiPlugin;

#[cfg(feature = "bevy")]
impl Plugin for ResetApiPlugin {
    fn build(&self, app: &mut App) {
        match zenoh::open(zenoh::Config::default()).wait() {
            Ok(session) => {
                let session = Arc::new(session);
                match ResetBroker::open(session) {
                    Ok(broker) => {
                        app.insert_resource(ResetApiSession {
                            broker: Mutex::new(broker),
                        });
                        app.add_systems(Update, drain_reset_inbox_system);
                        info!("gearbox-api: reset API ready (gearbox/sim/reset)");
                    }
                    Err(e) => {
                        warn!("gearbox-api: reset subscriber open failed ({e}); reset API disabled")
                    }
                }
            }
            Err(e) => warn!("gearbox-api: reset session open failed ({e}); reset API disabled"),
        }
    }
}

#[cfg(feature = "bevy")]
fn drain_reset_inbox_system(
    api: Option<Res<ResetApiSession>>,
    mut writer: MessageWriter<SimResetRequest>,
) {
    let Some(api) = api else { return };
    let Ok(broker) = api.broker.lock() else {
        return;
    };
    if let Some(req) = broker.take() {
        writer.write(SimResetRequest {
            pause_clock: req.pause_clock,
        });
    }
}
