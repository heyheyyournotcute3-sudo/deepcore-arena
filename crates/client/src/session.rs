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
            )
            // Intent sending runs after state updates so it always sees a
            // consistent view of the player and connection.
            .add_systems(Update, (send_move_input, send_mining_intents));
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
        // "Host game" runs the dedicated server as a child process and then
        // connects to it as a normal client. That keeps one authoritative
        // simulation implementation instead of a second, in-process copy that
        // could drift from the dedicated server.
        if request.host_server
            && let SessionMode::Client { addr, .. } = &request.mode
            && let Err(err) = spawn_hosted_server(addr.port(), &registry)
        {
            error!(%err, "could not start the hosted server");
            *state = ConnectionState::Failed(err);
            continue;
        }

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

/// Starts the dedicated server as a child process for "Host game".
///
/// The server binary sits next to the client binary in both `cargo run` and a
/// packaged build.
fn spawn_hosted_server(port: u16, registry: &ContentRegistry) -> Result<(), String> {
    let server_binary = std::env::current_exe()
        .map_err(|err| format!("cannot locate the client binary: {err}"))?
        .with_file_name(if cfg!(windows) {
            "server.exe"
        } else {
            "server"
        });
    if !server_binary.exists() {
        return Err(format!(
            "hosting needs the server binary next to the client ({} is missing); \
             run `cargo build -p deepcore-server` or use Join by IP",
            server_binary.display()
        ));
    }

    // Give the child the same content hash the client has, by pointing it at
    // the same content directory.
    let data_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/data");

    let mut command = std::process::Command::new(&server_binary);
    command
        .arg("--port")
        .arg(port.to_string())
        .arg("--data")
        .arg(data_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
        .spawn()
        .map_err(|err| format!("could not start {}: {err}", server_binary.display()))?;
    info!(port, "hosted server started");
    let _ = registry;
    Ok(())
}

/// Sends the handshake once the transport reports a connection.
fn send_hello(
    client: Option<Res<bevy_replicon_renet::RenetClient>>,
    hello: Option<Res<crate::net::PendingHello>>,
    mut out: MessageWriter<deepcore_shared::protocol::ClientHello>,
    state: Res<ConnectionState>,
    mut sent: Local<bool>,
) {
    let (Some(client), Some(hello)) = (client, hello) else {
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
    let _ = &client;
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

/// Sends the local player's movement input to the server each tick.
///
/// Movement is predicted locally; this is what lets the server stay authoritative
/// and send corrections back.
fn send_move_input(
    state: Res<ConnectionState>,
    mut out: MessageWriter<deepcore_shared::protocol::MoveInput>,
    player: Query<&crate::player::PlayerRuntime>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut seq: Local<u32>,
) {
    if *state != ConnectionState::Playing {
        return;
    }
    let Ok(runtime) = player.single() else {
        return;
    };

    let (dir, jump, sprint, crouch) = match keys {
        Some(keys) => {
            let mut dir = [0.0f32, 0.0f32];
            if keys.pressed(KeyCode::KeyW) {
                dir[1] += 1.0;
            }
            if keys.pressed(KeyCode::KeyS) {
                dir[1] -= 1.0;
            }
            if keys.pressed(KeyCode::KeyD) {
                dir[0] += 1.0;
            }
            if keys.pressed(KeyCode::KeyA) {
                dir[0] -= 1.0;
            }
            (
                dir,
                keys.pressed(KeyCode::Space),
                keys.pressed(KeyCode::ShiftLeft),
                keys.pressed(KeyCode::ControlLeft),
            )
        }
        // Headless clients send a gentle forward drift so the server has
        // something to simulate.
        None => ([0.0, 1.0], false, false, false),
    };

    *seq = seq.wrapping_add(1);
    out.write(deepcore_shared::protocol::MoveInput {
        seq: *seq,
        dir,
        jump,
        sprint,
        crouch,
        yaw: runtime.state.yaw,
        pitch: runtime.state.pitch,
    });
}

/// Sends mining start/stop intents when the player's mining state changes.
fn send_mining_intents(
    state: Res<ConnectionState>,
    mut start: MessageWriter<deepcore_shared::protocol::StartMining>,
    mut stop: MessageWriter<deepcore_shared::protocol::StopMining>,
    target: Option<Res<crate::player::TargetedBlock>>,
    player: Query<&crate::player::PlayerRuntime>,
    mut last_target: Local<Option<BlockPos>>,
) {
    if *state != ConnectionState::Playing {
        return;
    }
    let Some(target) = target else {
        return;
    };
    let Ok(runtime) = player.single() else {
        return;
    };

    let current = match (&target.0, runtime.mining.is_some()) {
        (Some(hit), true) => Some(hit.block),
        _ => None,
    };

    if current != *last_target {
        if last_target.is_some() && current.is_none() {
            stop.write(deepcore_shared::protocol::StopMining);
        }
        if let (Some(block), Some(hit)) = (current, target.0) {
            start.write(deepcore_shared::protocol::StartMining {
                block,
                face: wire_face(hit.face),
            });
        }
        *last_target = current;
    }
}

/// Converts the shared face type to the wire type.
fn wire_face(face: deepcore_shared::mining::Face) -> deepcore_shared::protocol::FaceWire {
    match face {
        deepcore_shared::mining::Face::West => deepcore_shared::protocol::FaceWire::West,
        deepcore_shared::mining::Face::East => deepcore_shared::protocol::FaceWire::East,
        deepcore_shared::mining::Face::Down => deepcore_shared::protocol::FaceWire::Down,
        deepcore_shared::mining::Face::Up => deepcore_shared::protocol::FaceWire::Up,
        deepcore_shared::mining::Face::North => deepcore_shared::protocol::FaceWire::North,
        deepcore_shared::mining::Face::South => deepcore_shared::protocol::FaceWire::South,
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
