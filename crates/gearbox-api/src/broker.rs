//! Pure-Rust zenoh broker. No Bevy dep. Owns the zenoh session and
//! every long-lived handle; drop it and the network surface
//! disappears.
//!
//! Reminder on scope: this is the **tool-API** boundary — zenoh
//! pub/sub/query for external robots, CLIs, agents. It is *not* the
//! simulator↔renderer transport. See the crate-level doc for the
//! layer separation.
//!
//! Only the sim-clock topic is wired today. Scene / asset topics
//! belong with the OpenUSD integration and will be authored there,
//! keyed by `sdf::Path`, when that lands.

use std::sync::{Arc, Mutex};

use zenoh::Wait;

use crate::wire::{ClockCommand, ClockWire, decode, encode};

// ─── Topic keys ─────────────────────────────────────────────────────
//
// Kept as `&'static str` constants because (a) Zenoh needs 'static
// key expressions for cached publishers and (b) having every key in
// one place makes it trivial to list the full API surface from a
// single `grep`.

const KEY_CLOCK_PUB: &str = "gearbox/sim/clock";
const KEY_CLOCK_COMMAND: &str = "gearbox/sim/clock/command";

// ─── Broker ─────────────────────────────────────────────────────────

pub struct ApiBroker {
    // `_session` is kept alive so the declared publisher / subscriber
    // handles remain valid; zenoh 1.x holds its own internal Arc so
    // we don't strictly need our own clone, but this makes the
    // ownership story unambiguous.
    _session: Arc<zenoh::Session>,

    clock_publisher: zenoh::pubsub::Publisher<'static>,
    pending_commands: Arc<Mutex<Vec<ClockCommand>>>,
    _clock_command_sub: zenoh::pubsub::Subscriber<()>,
}

impl ApiBroker {
    /// Open a default-config zenoh session (peer mode, local
    /// discovery) and declare every topic against it.
    pub fn open() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let session = Arc::new(zenoh::open(zenoh::Config::default()).wait()?);

        // Cached publisher: declares the key expression once and
        // reuses it on every `put`. Saves the per-call resolution
        // cost `session.put` would otherwise pay.
        let clock_publisher = session.declare_publisher(KEY_CLOCK_PUB).wait()?;

        // Control path: subscriber callback runs on a zenoh worker
        // thread, so it can't touch anything that isn't `Send +
        // Sync`. We buffer into a `Mutex<Vec<…>>` and let the owner
        // drain on its own thread.
        let pending: Arc<Mutex<Vec<ClockCommand>>> = Arc::new(Mutex::new(Vec::new()));
        let pending_cb = Arc::clone(&pending);
        let clock_command_sub = session
            .declare_subscriber(KEY_CLOCK_COMMAND)
            .callback(move |sample| {
                let bytes = sample.payload().to_bytes();
                match decode::<ClockCommand>(bytes.as_ref()) {
                    Ok(cmd) => {
                        if let Ok(mut q) = pending_cb.lock() {
                            q.push(cmd);
                        }
                    }
                    Err(e) => {
                        eprintln!("gearbox-api: bad clock command payload: {e}");
                    }
                }
            })
            .wait()?;

        Ok(Self {
            _session: session,
            clock_publisher,
            pending_commands: pending,
            _clock_command_sub: clock_command_sub,
        })
    }

    /// Drain every clock command received since the last call.
    /// Single-consumer by contract — whichever owner holds the
    /// clock state applies them.
    pub fn drain_clock_commands(&self) -> Vec<ClockCommand> {
        match self.pending_commands.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => Vec::new(),
        }
    }

    /// Publish the current clock state. Fire-and-forget; a missed
    /// publish is never worth a panic, so we log and move on.
    pub fn publish_clock(&self, clock: &ClockWire) {
        let Ok(bytes) = encode(clock) else { return };
        if let Err(e) = self.clock_publisher.put(bytes).wait() {
            eprintln!("gearbox-api: clock publish failed: {e}");
        }
    }
}
