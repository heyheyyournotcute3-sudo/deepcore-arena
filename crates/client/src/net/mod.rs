//! Client networking: transport startup, handshake, and reception of
//! authoritative state.
//!
//! The client predicts only its own movement. Everything else — chunks, block
//! changes, other players — arrives from the server and is applied as-is.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::SystemTime;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::netcode::{ClientAuthentication, NetcodeClientTransport};
use bevy_replicon_renet::renet::ConnectionConfig;
use bevy_replicon_renet::{RenetChannelsExt, RenetClient, RepliconRenetPlugins};

use deepcore_shared::protocol::{
    BlockChangeBatch, ChatMessage, ChunkBatch, ChunkUnload, ClientHello, MoveCorrection,
    PROTOCOL_VERSION, ServerHello,
};

/// Where to connect, or whether to host locally.
#[derive(Resource, Debug, Clone, PartialEq)]
pub enum SessionMode {
    /// Play alone with a locally simulated world (M1 behaviour).
    SinglePlayer,
    /// Connect to a server.
    Client {
        /// Server address.
        addr: SocketAddr,
        /// Name to show to other players.
        name: String,
    },
}

/// Connection status shown by the loading screen.
#[derive(Resource, Debug, Clone, PartialEq)]
pub enum ConnectionState {
    /// Not connected and not connecting.
    Idle,
    /// Handshake sent, waiting for the server's answer.
    Connecting,
    /// Accepted; streaming chunks.
    Loading {
        /// Chunks received so far.
        received: u32,
    },
    /// Fully in the world.
    Playing,
    /// Refused or disconnected, with a reason to show the player.
    Failed(String),
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self::Idle
    }
}

/// The player's server-assigned identity.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct LocalIdentity {
    /// Numeric player id from the server.
    pub player_id: u64,
    /// World seed the server is using.
    pub world_seed: u64,
}

/// Reconciliation state: the authoritative position and the inputs still in flight.
#[derive(Resource, Default)]
pub struct Prediction {
    /// Unacknowledged inputs, oldest first.
    pub pending: Vec<deepcore_shared::protocol::MoveInput>,
    /// Latest authoritative position from the server.
    pub authoritative: Option<MoveCorrection>,
    /// Corrections applied since the last report, for the debug overlay.
    pub corrections: u32,
    /// Smoothing offset currently being blended out, in blocks.
    pub smoothing_offset: Vec3,
}

/// How long to blend a small correction instead of snapping.
pub const SMOOTHING_SECS: f32 = 0.1;
/// Corrections smaller than this are smoothed; larger ones snap.
pub const SMOOTHING_THRESHOLD: f32 = 0.3;

/// Connects to a server and registers every wire type.
pub struct NetClientPlugin;

impl Plugin for NetClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((RepliconPlugins, RepliconRenetPlugins))
            .init_resource::<ConnectionState>()
            .init_resource::<LocalIdentity>()
            .init_resource::<Prediction>()
            .add_client_message::<ClientHello>(Channel::Ordered)
            .add_server_message::<ServerHello>(Channel::Ordered)
            .add_client_message::<deepcore_shared::protocol::MoveInput>(Channel::Unordered)
            .add_client_message::<deepcore_shared::protocol::StartMining>(Channel::Ordered)
            .add_client_message::<deepcore_shared::protocol::StopMining>(Channel::Ordered)
            .add_client_message::<deepcore_shared::protocol::PlaceBlock>(Channel::Ordered)
            .add_server_message::<BlockChangeBatch>(Channel::Ordered)
            .add_server_message::<ChunkBatch>(Channel::Ordered)
            .add_server_message::<ChunkUnload>(Channel::Ordered)
            .add_server_message::<MoveCorrection>(Channel::Unordered)
            .add_server_message::<deepcore_shared::protocol::MiningRefused>(Channel::Ordered)
            .add_server_message::<ChatMessage>(Channel::Ordered)
            .add_systems(Update, (handle_server_hello, handle_corrections));
    }
}

/// Starts the client transport toward `addr`.
///
/// Returns a `Result` so a bad address or an unavailable port surfaces as a
/// message on the loading screen instead of a panic.
pub fn start_client(
    commands: &mut Commands,
    channels: &RepliconChannels,
    addr: SocketAddr,
    content_hash: u64,
    name: &str,
    protocol_id: u64,
) -> Result<(), String> {
    // Match the server's enlarged bulk-channel budget; a client whose receive
    // budget is smaller will still drop connections while loading.
    let mut server_channels_config = channels.server_configs();
    for config in &mut server_channels_config {
        config.max_memory_usage_bytes = 32 * 1024 * 1024;
    }
    let mut client_channels_config = channels.client_configs();
    for config in &mut client_channels_config {
        config.max_memory_usage_bytes = 32 * 1024 * 1024;
    }

    let client = RenetClient::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|err| format!("system clock is before the unix epoch: {err}"))?;
    // netcode keys sessions by client id. Two clients started in the same
    // millisecond would collide, and the server would evict the first one with
    // "no private key was found for this address". Mix in the pid and a random
    // component so ids are unique per process.
    let client_id = (current_time.as_millis() as u64) << 20
        ^ ((std::process::id() as u64) << 8)
        ^ (rand_seed() & 0xFF);

    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .map_err(|err| format!("could not open a local UDP socket: {err}"))?;

    let authentication = ClientAuthentication::Unsecure {
        client_id,
        protocol_id,
        server_addr: addr,
        user_data: None,
    };
    let transport = NetcodeClientTransport::new(current_time, authentication, socket)
        .map_err(|err| format!("could not create the client transport: {err}"))?;

    commands.insert_resource(client);
    commands.insert_resource(transport);
    commands.insert_resource(PendingHello {
        content_hash,
        name: name.to_string(),
    });
    Ok(())
}

/// A cheap per-process random seed, for client id uniqueness.
fn rand_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(std::process::id() as u64);
    hasher.write_u128(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    hasher.finish()
}

/// The opening message, sent once the client reports it is connected.
#[derive(Resource, Debug, Clone)]
pub struct PendingHello {
    /// Hash of this client's content.
    pub content_hash: u64,
    /// Display name.
    pub name: String,
}

/// Sends the handshake as soon as the transport connects.
pub fn send_hello_when_connected(
    mut client: ResMut<RenetClient>,
    hello: Option<Res<PendingHello>>,
    mut out: MessageWriter<ClientHello>,
    mut state: ResMut<ConnectionState>,
    mut sent: Local<bool>,
) {
    let Some(hello) = hello else {
        return;
    };
    if !client.is_connected() {
        if *state == ConnectionState::Idle {
            *state = ConnectionState::Connecting;
        }
        return;
    }
    if *sent {
        return;
    }
    *sent = true;
    out.write(ClientHello {
        protocol_version: PROTOCOL_VERSION,
        content_hash: hello.content_hash,
        display_name: hello.name.clone(),
    });
    info!("handshake sent");
}

/// Applies the server's answer to the handshake.
pub fn handle_server_hello(
    mut messages: MessageReader<ServerHello>,
    mut state: ResMut<ConnectionState>,
    mut identity: ResMut<LocalIdentity>,
) {
    for message in messages.read() {
        match message {
            ServerHello::Accepted {
                player_id,
                world_seed,
                server_tick,
            } => {
                info!(
                    player_id,
                    world_seed, server_tick, "server accepted the connection"
                );
                identity.player_id = *player_id;
                identity.world_seed = *world_seed;
                *state = ConnectionState::Loading { received: 0 };
            }
            ServerHello::Refused { reason } => {
                // This is the path that shows "Content mismatch" to the player.
                warn!(%reason, "server refused the connection");
                *state = ConnectionState::Failed(reason.clone());
            }
        }
    }
}

/// Applies authoritative position corrections from the server.
pub fn handle_corrections(
    mut messages: MessageReader<MoveCorrection>,
    mut prediction: ResMut<Prediction>,
    mut player: Query<&mut crate::player::PlayerRuntime>,
) {
    for correction in messages.read() {
        // Drop acknowledged inputs.
        prediction
            .pending
            .retain(|input| input.seq > correction.last_processed_seq);
        prediction.authoritative = Some(*correction);

        let Ok(mut runtime) = player.single_mut() else {
            continue;
        };
        let server_pos = Vec3::from_array(correction.position);
        let error = server_pos - runtime.state.position;
        prediction.corrections += 1;

        if error.length() <= SMOOTHING_THRESHOLD {
            // Small disagreement: keep the predicted position and blend the
            // difference out over the smoothing window instead of snapping.
            prediction.smoothing_offset = error;
        } else {
            // Large disagreement (teleport, respawn, or a real desync): snap.
            runtime.state.position = server_pos;
            runtime.state.velocity = Vec3::from_array(correction.velocity);
            runtime.state.on_ground = correction.on_ground;
            prediction.smoothing_offset = Vec3::ZERO;
        }
    }
}

/// Blends out any smoothing offset, so corrections under 0.3 blocks are
/// invisible rather than jarring.
pub fn apply_smoothing(time: Res<Time>, mut prediction: ResMut<Prediction>) {
    if prediction.smoothing_offset == Vec3::ZERO {
        return;
    }
    let decay = (time.delta_secs() / SMOOTHING_SECS).clamp(0.0, 1.0);
    prediction.smoothing_offset *= 1.0 - decay;
    if prediction.smoothing_offset.length() < 1e-3 {
        prediction.smoothing_offset = Vec3::ZERO;
    }
}

/// Parses `host:port`, defaulting the port when only an IP is given.
pub fn parse_server_addr(text: &str, default_port: u16) -> Result<SocketAddr, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter a server address".to_string());
    }
    if let Ok(addr) = text.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Ok(ip) = text.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, default_port));
    }
    // Allow "localhost".
    if text.eq_ignore_ascii_case("localhost") {
        return Ok(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            default_port,
        ));
    }
    Err(format!(
        "{text:?} is not a valid address (try 127.0.0.1:43210)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_parse_with_and_without_a_port() {
        let with_port = parse_server_addr("192.168.1.5:5000", 43210).expect("valid");
        assert_eq!(with_port.port(), 5000);
        let without = parse_server_addr("192.168.1.5", 43210).expect("valid");
        assert_eq!(
            without.port(),
            43210,
            "the default port should be filled in"
        );
        let local = parse_server_addr("localhost", 43210).expect("valid");
        assert_eq!(local.ip().to_string(), "127.0.0.1");
    }

    #[test]
    fn bad_addresses_are_reported() {
        assert!(parse_server_addr("", 43210).is_err());
        assert!(parse_server_addr("not an address", 43210).is_err());
        // The message should suggest a working format.
        let err = parse_server_addr("nope", 43210).expect_err("bad");
        assert!(err.contains("127.0.0.1"), "unhelpful message: {err}");
    }

    #[test]
    fn small_corrections_are_smoothed_and_large_ones_snap() {
        // This mirrors the branch in `handle_corrections`.
        let error = Vec3::new(0.1, 0.0, 0.05);
        assert!(
            error.length() <= SMOOTHING_THRESHOLD,
            "small error should smooth"
        );
        let error = Vec3::new(2.0, 0.0, 0.0);
        assert!(
            error.length() > SMOOTHING_THRESHOLD,
            "large error should snap"
        );
    }

    #[test]
    fn smoothing_decays_to_zero() {
        let mut offset = Vec3::new(0.25, 0.0, 0.0);
        // Ten frames of 10 ms each is the full 100 ms window.
        for _ in 0..10 {
            let decay = (0.01 / SMOOTHING_SECS).clamp(0.0, 1.0);
            offset *= 1.0 - decay;
        }
        assert!(
            offset.length() < 1e-3,
            "offset should have decayed, got {offset:?}"
        );
    }
}
