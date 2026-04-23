//! Wire-format types, encoded with CBOR.
//!
//! Vehicle-specific types are intentionally absent: the scene /
//! asset layer is being replaced by OpenUSD (see the sibling
//! `bevy_openusd` project) and any future USD-path-addressed topics
//! will live alongside it. This file now only carries data that's
//! scene-agnostic and useful regardless of how the scene graph is
//! authored — currently just the sim clock.

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Sim clock state. Published periodically from whichever process
/// owns the `Sim`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClockWire {
    pub paused: bool,
    /// Simulation speed multiplier (`1.0` = real-time). Clients that
    /// only know about a discrete set of speeds should snap locally.
    pub speed: f32,
}

/// Command sent from a client to change the clock's state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ClockCommand {
    SetPaused(bool),
    SetSpeed(f32),
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, EncodeError> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf).map_err(EncodeError)?;
    Ok(buf)
}

pub(crate) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
    ciborium::from_reader(bytes).map_err(DecodeError)
}

// ─── Error wrappers ─────────────────────────────────────────────────
//
// ciborium's raw error types leak `std::io::Error` as the associated
// `E` parameter, which is noisy in signatures. Wrap once at the
// boundary and expose `Display`.

#[derive(Debug)]
pub struct EncodeError(ciborium::ser::Error<std::io::Error>);

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for EncodeError {}

#[derive(Debug)]
pub struct DecodeError(ciborium::de::Error<std::io::Error>);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for DecodeError {}
