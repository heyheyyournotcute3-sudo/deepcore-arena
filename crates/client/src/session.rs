//! Client session wiring: reacting to the menu's choice, receiving chunks and
//! block changes from the server, and driving the loading screen.
//!
//! In multiplayer the client does **not** generate the world from a seed for the
//! chunks it plays in — it receives them. The seed is still used for a local
//! preview in single-player, which is why `WorldGen` remains present.

use bevy::prelude::*;

use deepcore_shared::content::ContentRegistry;
use deepcore_shared::protocol::{
    BlockChangeBatch, ChunkBatch, ChunkUnload, DEFAULT_PORT, PROTOCOL_VERSION,
};
use deepcore_shared::voxel::{BlockId, BlockPos, Chunk, ChunkPos};

use crate::net::{ConnectionState, LocalIdentity, SessionMode};
use crate::render::chunks::ClientChunks;
use crate::ui::menu::{AppScreen, StartSession};

/// How many chunks must arrive before the loading screen lifts.
const CHUNKS_BEFORE_PLAYING: u32 = 40;

/// The client's networking session plugin.
pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        // SessionPlugin owns this message so headless mode (which has no menu)
        // can still start a session from the command line.
        // `ClientChunks` is the client's copy of the world. It lives here rather
        // than in the render plugin so networking works headless too; the render
        // plugin only reads it.
        app.init_resource::<ClientChunks>()
            .add_message::<StartSession>()
            .add_systems(
                Update,
                (
                    handle_start_session,
                    send_hello,
                    receive_chunks,
                    receive_block_changes,
                    receive_chunk_unloads,
                    finish_loading,
                )
                    .chain(),
            );
    }
}

/// Starts the transport and/or local world when the player picks a mode.
fn handle_start_session(
    mut requests: MessageReader<StartSession>,
    mut commands: Commands,
    channels: Option<Res<bevy_replicon::prelude::RepliconChannels>>,
    registry: Option<Res<ContentRegistry>>,
    mut state: ResMut<ConnectionState>,
    mut chunks: ResMut<ClientChunks>,
) {
    let Some(channels) = channels else {
        return;
    };
    let Some(registry) = registry else {
        return;
    };

    for request in requests.read() {
        match &request.mode {
            SessionMode::SinglePlayer => {
                // No transport: the local generator feeds chunks directly.
                info!("starting single-player session");
                chunks.chunks.clear();
                *state = ConnectionState::Playing;
            }
            SessionMode::Client { addr, name } => {
                info!(%addr, %name, "connecting");
                chunks.chunks.clear();
                let content_hash = registry.0.content_hash();
                match crate::net::start_client(
                    &mut commands,
                    &channels,
                    *addr,
                    content_hash,
                    name,
                    crate::NET_PROTOCOL_ID,
                ) {
                    Ok(()) => {
                        *state = ConnectionState::Connecting;
                    }
                    Err(err) => {
                        error!(%err, "could not start the client");
                        *state = ConnectionState::Failed(err);
                    }
                }
                let _ = DEFAULT_PORT;
                let _ = PROTOCOL_VERSION;
            }
        }
    }
}

/// Sends the handshake once the transport reports a connection.
fn send_hello(
    client: Option<ResMut<bevy_replicon_renet::RenetClient>>,
    hello: Option<Res<crate::net::PendingHello>>,
    mut out: MessageWriter<deepcore_shared::protocol::ClientHello>,
    mut state: ResMut<ConnectionState>,
    mut sent: Local<bool>,
) {
    let (Some(mut client), Some(hello)) = (client, hello) else {
        return;
    };
    if *sent || !client.is_connected() {
        return;
    }
    *sent = true;
    out.write(deepcore_shared::protocol::ClientHello {
        protocol_version: PROTOCOL_VERSION,
        content_hash: hello.content_hash,
        display_name: hello.name.clone(),
    });
    if *state == ConnectionState::Connecting {
        debug!("handshake sent");
    }
    let _ = &mut client;
}

/// Applies chunk data sent by the server.
fn receive_chunks(
    mut messages: MessageReader<ChunkBatch>,
    mut chunks: ResMut<ClientChunks>,
    mut state: ResMut<ConnectionState>,
) {
    for batch in messages.read() {
        for chunk_data in &batch.chunks {
            let pos = ChunkPos::new(chunk_data.pos.0, chunk_data.pos.1, chunk_data.pos.2);
            match Chunk::from_bytes(pos, &chunk_data.data) {
                Ok(chunk) => {
                    chunks.chunks.insert(pos, chunk);
                }
                Err(err) => {
                    // A corrupt chunk must not kill the session, but it must be
                    // loud: this indicates a protocol or content problem.
                    error!(?pos, %err, "received a corrupt chunk");
                }
            }
        }
        if let ConnectionState::Loading { received } = &mut *state {
            *received += batch.chunks.len() as u32;
        }
    }
}

/// Applies authoritative block changes.
fn receive_block_changes(
    mut messages: MessageReader<BlockChangeBatch>,
    mut chunks: ResMut<ClientChunks>,
) {
    for batch in messages.read() {
        for change in &batch.changes {
            let pos = BlockPos::new(change.pos.x, change.pos.y, change.pos.z);
            chunks.set_block(pos, BlockId(change.block));
        }
    }
}

/// Drops chunks the server says are out of range.
fn receive_chunk_unloads(
    mut messages: MessageReader<ChunkUnload>,
    mut chunks: ResMut<ClientChunks>,
) {
    for unload in messages.read() {
        let pos = ChunkPos::new(unload.pos.0, unload.pos.1, unload.pos.2);
        chunks.chunks.remove(&pos);
    }
}

/// Lifts the loading screen once enough chunks have arrived.
fn finish_loading(
    mut state: ResMut<ConnectionState>,
    chunks: Res<ClientChunks>,
    // Headless mode has no UI state machine, so this is optional.
    next_screen: Option<ResMut<NextState<AppScreen>>>,
    identity: Res<LocalIdentity>,
) {
    if let ConnectionState::Loading { received } = &*state
        && (*received >= CHUNKS_BEFORE_PLAYING
            || chunks.chunks.len() >= CHUNKS_BEFORE_PLAYING as usize)
    {
        info!(
            chunks = chunks.chunks.len(),
            player_id = identity.player_id,
            "world loaded, entering play"
        );
        *state = ConnectionState::Playing;
        if let Some(mut next_screen) = next_screen {
            next_screen.set(AppScreen::Playing);
        }
    }
}

/// Reports the connection state in the HUD, so a stuck connection is visible.
pub fn status_text(state: &ConnectionState) -> String {
    match state {
        ConnectionState::Idle => String::new(),
        ConnectionState::Connecting => "Connecting…".to_string(),
        ConnectionState::Loading { received } => format!("Loading world… {received} chunks"),
        ConnectionState::Playing => String::new(),
        ConnectionState::Failed(reason) => format!("Connection failed: {reason}"),
    }
}
