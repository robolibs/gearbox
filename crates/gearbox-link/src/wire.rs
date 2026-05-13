//! Wire-format messages crossing the simulator ↔ renderer link.
//!
//! CBOR-encoded. Kept deliberately narrow — this is the boundary
//! between two trusted halves of our own stack, not an external
//! protocol. When scene data lands (OpenUSD) the message enum grows
//! new variants rather than a second topic surface.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

// ─── Simulator → Renderer ──────────────────────────────────────────

/// Messages the simulator sends to every attached renderer each
/// frame (or on-change, depending on the variant).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SimToRenderer {
    /// Per-frame heartbeat. Ships the authoritative sim time so
    /// client-side interpolation / prediction has a reference.
    Tick { time_s: f64, frame: u64 },
    /// Clock state mirror. Emitted on change (pause toggle, speed
    /// change) so a client UI can reflect the sim's current mode
    /// without polling the tool API.
    ClockState { paused: bool, speed: f32 },
}

// ─── Renderer → Simulator ──────────────────────────────────────────

/// Messages a renderer client sends back to the sim. Primarily
/// input and intent — "the user pressed W" — plus command-style
/// requests like pause.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RendererToSim {
    /// Round-trip liveness probe.
    Ping,
    /// Request the sim pause / resume.
    SetPaused(bool),
    /// Current teleop axes, all scaled to `-1.0..=1.0`. Ship every
    /// frame the values change; sim applies to the currently-active
    /// player-controlled vehicle.
    Input {
        throttle: f32,
        steer: f32,
        brake: f32,
        yaw: f32,
        lift: f32,
    },
}

// ─── Codec ─────────────────────────────────────────────────────────

/// Encode a message into CBOR bytes. Lossless; fine for 60 Hz.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, CodecError> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf).map_err(|e| CodecError(format!("encode: {e}")))?;
    Ok(buf)
}

/// Decode a message from CBOR bytes.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CodecError> {
    ciborium::from_reader(bytes).map_err(|e| CodecError(format!("decode: {e}")))
}

#[derive(Debug)]
pub struct CodecError(pub String);

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CodecError {}
