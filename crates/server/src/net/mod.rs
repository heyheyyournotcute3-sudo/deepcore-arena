//! Server networking: transport startup, handshake, and per-client state.
//!
//! The server is authoritative. It accepts connections, refuses mismatched
//! clients with a readable reason, and validates every intent before acting.
//!
//! Replicon 0.45 identifies clients by **entity** (`ClientId::Client(Entity)`),
//! not by a numeric id, so per-client state is keyed by entity and the numeric
//! `player_id` we send in the handshake is derived from it.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::SystemTime;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig};
use bevy_replicon_renet::renet::ConnectionConfig;
use bevy_replicon_renet::{RenetChannelsExt, RenetServer, RepliconRenetPlugins};

use deepcore_shared::protocol::{
    BlockChangeBatch, ChatMessage, ChunkBatch, ChunkUnload, ClientHello, MAX_PLAYERS,
    MoveCorrection, PROTOCOL_VERSION, ServerHello, check_handshake,
};

use crate::config::ServerConfig as DeepcoreConfig;

/// Shared netcode private key, matching the client's.
pub const PRIVATE_KEY: [u8; 32] = [0x0D; 32];

/// The server's content hash, used in the handshake.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ContentHash(pub u64);

/// A client that has been refused and should be disconnected shortly, once its
/// refusal message has had time to flush.
#[derive(Component, Debug, Clone, Copy)]
pub struct PendingDisconnect {
    /// Ticks remaining before the connection is dropped.
    pub after_ticks: u32,
}

/// Drops connections whose refusal has had time to reach the client.
pub fn process_pending_disconnects(
    mut commands: Commands,
    mut registry: ResMut<ClientRegistry>,
    mut pending: Query<(Entity, &mut PendingDisconnect)>,
) {
    for (entity, mut state) in &mut pending {
        if state.after_ticks == 0 {
            debug!(%entity, "dropping refused connection");
            commands.entity(entity).despawn();
            registry.clients.retain(|c| c.entity != entity);
        } else {
            state.after_ticks -= 1;
        }
    }
}

/// Per-client server-side state.
#[derive(Resource, Default)]
pub struct ClientRegistry {
    /// Connected clients.
    pub clients: Vec<ConnectedPlayer>,
}

/// One connected player as the server sees them.
#[derive(Debug, Clone)]
pub struct ConnectedPlayer {
    /// The client entity replicon uses to identify this connection.
    pub entity: Entity,
    /// Stable numeric id, sent to the client in the handshake.
    pub player_id: u64,
    /// The player's avatar entity in the world, once spawned.
    pub avatar: Option<Entity>,
    /// Display name from the handshake.
    pub name: String,
    /// Whether the handshake completed.
    pub accepted: bool,
    /// Chunks this client currently has, for computing unload messages.
    pub known_chunks: std::collections::HashSet<(i32, i32, i32)>,
}

impl ClientRegistry {
    /// Finds a player by client entity.
    pub fn by_entity(&self, entity: Entity) -> Option<&ConnectedPlayer> {
        self.clients.iter().find(|c| c.entity == entity)
    }

    /// Finds a player mutably by client entity.
    pub fn by_entity_mut(&mut self, entity: Entity) -> Option<&mut ConnectedPlayer> {
        self.clients.iter_mut().find(|c| c.entity == entity)
    }

    /// Number of accepted players.
    pub fn player_count(&self) -> usize {
        self.clients.iter().filter(|c| c.accepted).count()
    }
}

/// Starts the UDP server and inserts the renet resources.
///
/// Returns a `Result` rather than panicking: a server that cannot bind its port
/// should report why and exit, not crash.
pub fn start_transport(
    commands: &mut Commands,
    channels: &RepliconChannels,
    config: &DeepcoreConfig,
) -> Result<SocketAddr, String> {
    // Chunk streaming is bulk data over a reliable ordered channel. The default
    // 5 MB send-buffer budget is exhausted while a client loads its first
    // interest set, which disconnects the client mid-load. Give bulk channels a
    // much larger budget; the tick-rate limiter in `stream_chunks` still bounds
    // how much is queued at once.
    let mut server_channels_config = channels.server_configs();
    for config in &mut server_channels_config {
        config.max_memory_usage_bytes = 32 * 1024 * 1024;
    }
    let mut client_channels_config = channels.client_configs();
    for config in &mut client_channels_config {
        config.max_memory_usage_bytes = 32 * 1024 * 1024;
    }

    let server = RenetServer::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|err| format!("system clock is before the unix epoch: {err}"))?;

    let addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), config.port);
    let socket = UdpSocket::bind(addr).map_err(|err| format!("could not bind {addr}: {err}"))?;
    let bound = socket
        .local_addr()
        .map_err(|err| format!("could not read the bound address: {err}"))?;

    let server_config = ServerConfig {
        current_time,
        max_clients: MAX_PLAYERS,
        protocol_id: config.protocol_id,
        // Must match the client's key and auth mode.
        authentication: ServerAuthentication::Secure {
            private_key: PRIVATE_KEY,
        },
        // Secure connect tokens are validated against the server's advertised
        // addresses, so a token naming 127.0.0.1 must be matched by an
        // advertised 127.0.0.1. Listing the loopback address is what makes
        // local and LAN play work; a public server adds its own address here.
        public_addresses: vec![
            SocketAddr::new(Ipv4Addr::LOCALHOST.into(), config.port),
            SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), config.port),
        ],
    };
    let transport = NetcodeServerTransport::new(server_config, socket)
        .map_err(|err| format!("could not create the netcode transport: {err}"))?;

    commands.insert_resource(server);
    commands.insert_resource(transport);
    Ok(bound)
}

/// The server's networking plugin: registers every wire type.
pub struct NetServerPlugin;

impl Plugin for NetServerPlugin {
    fn build(&self, app: &mut App) {
        // RepliconPlugins must be added before any message registration: it
        // inserts the protocol hasher and remote-message registry that
        // `add_client_message` writes into.
        app.add_plugins((RepliconPlugins, RepliconRenetPlugins))
            .init_resource::<ClientRegistry>()
            // Replication rules. Without these, entities marked `Replicated`
            // are never actually sent to clients: the marker alone does nothing.
            .replicate::<deepcore_shared::protocol::PlayerTransform>()
            .replicate::<deepcore_shared::protocol::PlayerInfo>()
            .replicate::<deepcore_shared::protocol::Vitals>()
            // Hide each player's own avatar from themselves.
            .add_visibility_filter::<crate::sim::players::AvatarOwner>()
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
            .add_observer(on_client_connected)
            .add_systems(
                Update,
                (
                    handle_hello,
                    process_pending_disconnects,
                    handle_disconnects,
                    report_server_state,
                ),
            );
    }
}

/// Registers a newly connected client so it has a slot before the handshake.
///
/// `ConnectedClient` is a component in replicon 0.45, so this is an observer on
/// its insertion rather than a message reader.
fn on_client_connected(connected: On<Add<ConnectedClient>>, mut registry: ResMut<ClientRegistry>) {
    let entity = connected.entity;
    if registry.by_entity(entity).is_some() {
        return;
    }
    info!(%entity, "connection opened, awaiting handshake");
    registry.clients.push(ConnectedPlayer {
        entity,
        player_id: entity.to_bits() & 0xFFFF_FFFF,
        avatar: None,
        name: String::new(),
        accepted: false,
        known_chunks: Default::default(),
    });
}

/// Validates a client's opening message and accepts or refuses it.
///
/// The refusal reason is sent back before the connection is dropped, so the
/// player reads "Content mismatch" rather than seeing a bare disconnect.
pub fn handle_hello(
    mut messages: MessageReader<FromClient<ClientHello>>,
    mut out: MessageWriter<ToClients<ServerHello>>,
    mut commands: Commands,
    mut registry: ResMut<ClientRegistry>,
    content_hash: Option<Res<ContentHash>>,
    world_gen: Option<Res<deepcore_shared::worldgen::WorldGen>>,
    tick: Option<Res<crate::TickCounter>>,
) {
    let (Some(content_hash), Some(world_gen)) = (content_hash, world_gen) else {
        return;
    };

    let server_tick = tick.as_ref().map(|t| t.ticks).unwrap_or(0);
    for FromClient { client_id, message } in messages.read() {
        let Some(entity) = client_id.entity() else {
            // The server acting as its own client; nothing to hand-shake.
            continue;
        };

        match check_handshake(message, PROTOCOL_VERSION, content_hash.0) {
            None => {
                let name = message.display_name.trim().to_string();
                let player_id = registry
                    .by_entity(entity)
                    .map(|c| c.player_id)
                    .unwrap_or_else(|| entity.to_bits() & 0xFFFF_FFFF);
                info!(player_id, %name, "client accepted");
                if let Some(player) = registry.by_entity_mut(entity) {
                    player.name = name;
                    player.accepted = true;
                    player.player_id = player_id;
                }

                out.write(ToClients {
                    targets: SendTargets::Single(ClientId::Client(entity)),
                    message: ServerHello::Accepted {
                        player_id,
                        world_seed: world_gen.seed(),
                        server_tick,
                    },
                });
            }
            Some(reason) => {
                warn!(%reason, "refusing client");
                out.write(ToClients {
                    targets: SendTargets::Single(ClientId::Client(entity)),
                    message: ServerHello::Refused {
                        reason: reason.clone(),
                    },
                });
                // Give the refusal a moment to reach the client before dropping
                // the connection. Disconnecting in the same tick loses the
                // message, and the player then sees a bare disconnect instead of
                // "Content mismatch".
                commands
                    .entity(entity)
                    .insert(PendingDisconnect { after_ticks: 15 });
            }
        }
    }
}

/// Logs disconnections and forgets the player.
pub fn handle_disconnects(
    mut registry: ResMut<ClientRegistry>,
    mut disconnected: MessageReader<DisconnectRequest>,
) {
    for request in disconnected.read() {
        if let Some(player) = registry.by_entity(request.client) {
            warn!(
                name = %player.name,
                accepted = player.accepted,
                entity = %request.client,
                "client disconnected"
            );
        }
        registry.clients.retain(|c| c.entity != request.client);
    }
}

/// Reports the replicon server state each second, so a state stuck outside
/// `Running` (which silently drops all traffic) is obvious in the log.
pub fn report_server_state(
    state: Option<Res<State<ServerState>>>,
    mut last: Local<Option<std::time::Instant>>,
) {
    let now = std::time::Instant::now();
    let due = last
        .map(|i| now.duration_since(i).as_secs_f32() >= 2.0)
        .unwrap_or(true);
    if !due {
        return;
    }
    *last = Some(now);
    match state {
        Some(state) => debug!(state = ?state.get(), "replicon server state"),
        None => debug!("replicon server state resource missing"),
    }
}

/// Number of live connections reported by the transport.
pub fn connected_count(server: &RenetServer) -> usize {
    server.clients_id().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(entity_bits: u32, name: &str, accepted: bool) -> ConnectedPlayer {
        ConnectedPlayer {
            entity: Entity::from_bits(entity_bits as u64),
            player_id: entity_bits as u64,
            avatar: None,
            name: name.to_string(),
            accepted,
            known_chunks: Default::default(),
        }
    }

    #[test]
    fn registry_counts_only_accepted_players() {
        let mut registry = ClientRegistry::default();
        registry.clients.push(player(1, "a", true));
        registry.clients.push(player(2, "b", false));
        assert_eq!(registry.player_count(), 1);
    }

    #[test]
    fn registry_lookups_by_entity_and_id() {
        let mut registry = ClientRegistry::default();
        registry.clients.push(player(7, "seven", true));
        assert!(registry.by_entity(Entity::from_bits(7)).is_some());
        assert!(registry.by_entity(Entity::from_bits(8)).is_none());
        assert!(registry.by_entity_mut(Entity::from_bits(7)).is_some());
    }

    #[test]
    fn known_chunks_start_empty() {
        let p = player(1, "a", true);
        assert!(p.known_chunks.is_empty());
    }
}
