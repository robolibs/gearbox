//! Client side — runs inside the renderer process (native or wasm).
//! Opens a WebTransport connection to the simulator, publishes
//! every incoming packet as a Bevy `IncomingFrame`, and encodes
//! every `OutgoingInput` onto the wire.
//!
//! Dev posture on native: trust any cert (including the self-signed
//! one the server hands out), via wtransport's `dangerous-
//! configuration`. Wasm clients don't see this flag — the browser
//! handles trust via `serverCertificateHashes` in `ClientConfig`,
//! which we don't populate yet. That's a follow-up for the wasm
//! renderer build; the message-layer plumbing here doesn't care.

use aeronet_webtransport::client::{ClientConfig, WebTransportClient, WebTransportClientPlugin};
use aeronet_webtransport::session::WebTransportIo;
use aeronet::io::Session;
use bevy::prelude::*;
use bytes::Bytes;

use crate::wire::{decode, encode, RendererToSim, SimToRenderer};

/// URL of the simulator's WebTransport endpoint.
#[derive(Resource, Debug, Clone)]
pub struct LinkClientConfig {
    pub server_url: String,
}

impl Default for LinkClientConfig {
    fn default() -> Self {
        // Matches `LinkServerConfig`'s default bind port.
        Self { server_url: "https://127.0.0.1:7824".into() }
    }
}

/// A [`SimToRenderer`] message that arrived over the wire. Consume
/// with `MessageReader<IncomingFrame>`.
#[derive(Message, Debug, Clone)]
pub struct IncomingFrame(pub SimToRenderer);

/// A [`RendererToSim`] message to send on the next link tick. Emit
/// with `MessageWriter<OutgoingInput>`.
#[derive(Message, Debug, Clone, Copy)]
pub struct OutgoingInput(pub RendererToSim);

pub struct LinkClientPlugin;

impl Plugin for LinkClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LinkClientConfig>()
            .add_message::<IncomingFrame>()
            .add_message::<OutgoingInput>()
            .add_plugins(WebTransportClientPlugin)
            .add_systems(Startup, connect_to_server)
            .add_systems(
                PreUpdate,
                drain_incoming_packets_system.after(aeronet::io::IoSystems::Poll),
            )
            .add_systems(PostUpdate, send_outgoing_inputs_system);
    }
}

// ─── Session lifecycle ─────────────────────────────────────────────

fn connect_to_server(mut commands: Commands, cfg: Res<LinkClientConfig>) {
    let client_config = build_client_config();
    commands
        .spawn((Name::new("LinkClient"),))
        .queue(WebTransportClient::connect(client_config, cfg.server_url.clone()));
    info!("link-client: dialing {}", cfg.server_url);
}

#[cfg(not(target_family = "wasm"))]
fn build_client_config() -> ClientConfig {
    use aeronet_webtransport::wtransport::ClientConfig as WtClientConfig;
    // Native dev: accept the server's self-signed cert. Do NOT ship
    // this in a release — it disables cert validation for every
    // connection.
    WtClientConfig::builder()
        .with_bind_default()
        .with_no_cert_validation()
        .build()
}

#[cfg(target_family = "wasm")]
fn build_client_config() -> ClientConfig {
    // Browser client. Trust is enforced via the server-cert-hash
    // list we expect to add once the wasm renderer lands; an empty
    // `ClientConfig` leaves trust up to the browser's WebPKI.
    ClientConfig::default()
}

// ─── Per-frame I/O pumping ─────────────────────────────────────────

fn drain_incoming_packets_system(
    mut sessions: Query<&mut Session, With<WebTransportIo>>,
    mut incoming: MessageWriter<IncomingFrame>,
) {
    for mut session in &mut sessions {
        for packet in session.recv.drain(..) {
            match decode::<SimToRenderer>(&packet.payload) {
                Ok(msg) => {
                    incoming.write(IncomingFrame(msg));
                }
                Err(e) => {
                    warn!("link-client: bad inbound payload: {e}");
                }
            }
        }
    }
}

fn send_outgoing_inputs_system(
    mut outgoing: MessageReader<OutgoingInput>,
    mut sessions: Query<&mut Session, With<WebTransportIo>>,
) {
    let encoded: Vec<Bytes> = outgoing
        .read()
        .filter_map(|o| encode(&o.0).ok().map(Bytes::from))
        .collect();
    if encoded.is_empty() {
        return;
    }
    // Typical client has exactly one session; loop in case a future
    // multi-home setup is ever grafted on.
    for mut session in &mut sessions {
        for bytes in &encoded {
            session.send.push(bytes.clone());
        }
    }
}
