//! Server side — runs inside the simulator process. Listens on a
//! UDP/QUIC port via aeronet's WebTransport IO layer, accepts every
//! incoming client, streams them [`SimToRenderer`] frames, and
//! forwards [`RendererToSim`] messages back into the sim as Bevy
//! `IncomingInput` messages.
//!
//! Dev posture: self-signed cert on a fixed port, and we accept
//! every client that tries to connect. Production will want real
//! WebPKI certs plus an auth hook on `SessionRequest`.

use aeronet_webtransport::server::{
    ServerConfig, SessionRequest, SessionResponse, WebTransportServer, WebTransportServerPlugin,
};
use aeronet_webtransport::wtransport::Identity;
use aeronet_webtransport::session::WebTransportIo;
use aeronet::io::Session;
use bevy::prelude::*;
use bytes::Bytes;

use crate::wire::{decode, encode, RendererToSim, SimToRenderer};

/// Bind config for the QUIC listener.
#[derive(Resource, Debug, Clone)]
pub struct LinkServerConfig {
    pub bind_port: u16,
    /// SAN entries the self-signed cert will cover. Keep the set
    /// small — renderer clients need to reach the server at a name
    /// in this list or TLS will refuse.
    pub san: Vec<String>,
}

impl Default for LinkServerConfig {
    fn default() -> Self {
        Self {
            bind_port: 7824,
            san: vec!["localhost".into(), "127.0.0.1".into(), "::1".into()],
        }
    }
}

/// A [`RendererToSim`] message that arrived over the wire. Consume
/// with `MessageReader<IncomingInput>`.
#[derive(Message, Debug, Clone, Copy)]
pub struct IncomingInput(pub RendererToSim);

/// A [`SimToRenderer`] message to broadcast to every connected
/// renderer. Emit with `MessageWriter<OutgoingFrame>`.
#[derive(Message, Debug, Clone)]
pub struct OutgoingFrame(pub SimToRenderer);

pub struct LinkServerPlugin;

impl Plugin for LinkServerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LinkServerConfig>()
            .add_message::<IncomingInput>()
            .add_message::<OutgoingFrame>()
            .add_plugins(WebTransportServerPlugin)
            .add_observer(accept_all_connections)
            .add_systems(Startup, open_server)
            .add_systems(
                PreUpdate,
                drain_incoming_packets_system.after(aeronet::io::IoSystems::Poll),
            )
            .add_systems(PostUpdate, broadcast_outgoing_frames_system);
    }
}

// ─── Session lifecycle ─────────────────────────────────────────────

fn open_server(mut commands: Commands, cfg: Res<LinkServerConfig>) {
    // Self-signed identity valid for the configured SANs. Cheap
    // to regenerate each run; browser clients wanting `wss` style
    // trust must pin the cert hash separately.
    let identity = match Identity::self_signed(cfg.san.iter().map(String::as_str)) {
        Ok(id) => id,
        Err(e) => {
            warn!("link-server: self-signed cert failed ({e}); link disabled");
            return;
        }
    };
    let server_config = ServerConfig::builder()
        .with_bind_default(cfg.bind_port)
        .with_identity(identity)
        .build();
    commands
        .spawn((Name::new("LinkServer"),))
        .queue(WebTransportServer::open(server_config));
    info!(
        "link-server: listening on udp/{} for simulator↔renderer clients",
        cfg.bind_port
    );
}

/// Aeronet delivers a `SessionRequest` trigger on every new client;
/// we respond with `Accepted` unconditionally (dev posture).
fn accept_all_connections(mut request: On<SessionRequest>) {
    request.respond(SessionResponse::Accepted);
}

// ─── Per-frame I/O pumping ─────────────────────────────────────────

/// Drains every connected client's `Session::recv` buffer, decodes
/// each packet as a [`RendererToSim`] message, and re-emits it as an
/// `IncomingInput` so the rest of the app can subscribe normally.
fn drain_incoming_packets_system(
    mut sessions: Query<&mut Session, With<WebTransportIo>>,
    mut incoming: MessageWriter<IncomingInput>,
) {
    for mut session in &mut sessions {
        for packet in session.recv.drain(..) {
            match decode::<RendererToSim>(&packet.payload) {
                Ok(msg) => {
                    incoming.write(IncomingInput(msg));
                }
                Err(e) => {
                    warn!("link-server: bad inbound payload: {e}");
                }
            }
        }
    }
}

/// Serialises every queued [`OutgoingFrame`] and pushes the bytes
/// onto every connected client's `Session::send` buffer. Aeronet's
/// IO poll-phase drains that buffer over the wire.
fn broadcast_outgoing_frames_system(
    mut frames: MessageReader<OutgoingFrame>,
    mut sessions: Query<&mut Session, With<WebTransportIo>>,
) {
    // Encode once, clone the `Bytes` into each session — Bytes uses
    // ref-counted storage so the payload is shared, not copied.
    let encoded: Vec<Bytes> = frames
        .read()
        .filter_map(|f| encode(&f.0).ok().map(Bytes::from))
        .collect();
    if encoded.is_empty() {
        return;
    }
    for mut session in &mut sessions {
        for bytes in &encoded {
            session.send.push(bytes.clone());
        }
    }
}
