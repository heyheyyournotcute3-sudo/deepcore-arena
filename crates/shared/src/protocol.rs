//! The network protocol: client intents, server messages and replicated state.
//!
//! Every type here is part of the wire format. **Bump [`PROTOCOL_VERSION`]
//! whenever a shape in this file changes**, or a stale client will mis-deserialise
//! instead of being cleanly refused.
//!
//! Direction matters:
//! - *Intents* travel client → server. The server validates every one of them.
//! - *Messages* travel server → client (or client → server for the handshake).
//! - *Replicated components* are owned by the server and mirrored on clients.

use bevy::ecs::message::Message;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::voxel::BlockPos;

/// Network protocol version. Must match exactly between client and server.
pub const PROTOCOL_VERSION: u32 = 2;

/// Default UDP port for a hosted game.
pub const DEFAULT_PORT: u16 = 43210;

/// Maximum players in a co-op session.
pub const MAX_PLAYERS: usize = 16;

/// Server simulation rate. Both sides must agree on this for mining timers.
pub const TICK_HZ: u32 = 30;

/// Horizontal interest-management radius, in chunks.
pub const VIEW_RADIUS_CHUNKS_XZ: i32 = 6;

/// Vertical interest-management radius, in chunks.
pub const VIEW_RADIUS_CHUNKS_Y: i32 = 4;

/// Maximum reach for mining and placing, in blocks.
pub const REACH: f32 = 5.0;

/// Maximum reach with an area drill, in blocks.
pub const REACH_DRILL: f32 = 6.0;

// ---------------------------------------------------------------- handshake

/// Sent by the client as its very first message.
#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct ClientHello {
    /// Protocol version the client was built with.
    pub protocol_version: u32,
    /// Hash of the client's resolved content, compared with the server's.
    pub content_hash: u64,
    /// Display name shown to other players.
    pub display_name: String,
}

/// Sent by the server to accept or refuse a connection.
#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub enum ServerHello {
    /// The connection is accepted.
    Accepted {
        /// The player's server-assigned id.
        player_id: u64,
        /// The world seed, so the client can generate terrain.
        world_seed: u64,
        /// Current server tick, so the client can align its clock.
        server_tick: u64,
    },
    /// The connection is refused, with a reason shown to the player.
    Refused {
        /// Human-readable reason, e.g. "Content mismatch".
        reason: String,
    },
}

/// Builds the refusal reason for a handshake mismatch, or `None` if compatible.
///
/// Kept as one function so the client and the server can never disagree about
/// what counts as a mismatch.
pub fn check_handshake(
    client: &ClientHello,
    server_protocol: u32,
    server_content_hash: u64,
) -> Option<String> {
    if client.protocol_version != server_protocol {
        return Some(format!(
            "Protocol mismatch: server is v{server_protocol}, you are v{}",
            client.protocol_version
        ));
    }
    if client.content_hash != server_content_hash {
        return Some("Content mismatch".to_string());
    }
    if client.display_name.trim().is_empty() {
        return Some("Empty display name".to_string());
    }
    if client.display_name.chars().count() > 32 {
        return Some("Display name too long (max 32 characters)".to_string());
    }
    None
}

// ------------------------------------------------------------- client intent

/// A movement input for one fixed tick, sent unreliably and sequenced.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Message)]
pub struct MoveInput {
    /// Monotonic sequence number, echoed back by the server for reconciliation.
    pub seq: u32,
    /// Local movement direction: x = right, z = forward.
    pub dir: [f32; 2],
    /// Jump held.
    pub jump: bool,
    /// Sprint held.
    pub sprint: bool,
    /// Crouch held.
    pub crouch: bool,
    /// Look yaw in radians.
    pub yaw: f32,
    /// Look pitch in radians.
    pub pitch: f32,
}

/// Start mining a block.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Message)]
pub struct StartMining {
    /// The block being targeted.
    pub block: BlockPos,
    /// The face that was targeted.
    pub face: FaceWire,
}

/// Stop the current mining operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Message)]
pub struct StopMining;

/// Place the held item as a block against a face.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Message)]
pub struct PlaceBlock {
    /// The block that was targeted.
    pub target: BlockPos,
    /// The face that was targeted; the new block goes in front of it.
    pub face: FaceWire,
    /// Which hotbar slot supplies the item.
    pub item_slot: u8,
}

/// Interact with a machine or structure.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Message)]
pub struct Interact {
    /// The block that was targeted.
    pub target: BlockPos,
}

/// The six block faces, as they travel over the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FaceWire {
    /// Negative X.
    West,
    /// Positive X.
    East,
    /// Negative Y.
    Down,
    /// Positive Y.
    Up,
    /// Negative Z.
    North,
    /// Positive Z.
    South,
}

impl FaceWire {
    /// The outward normal of the face.
    pub fn normal(self) -> (i32, i32, i32) {
        match self {
            FaceWire::West => (-1, 0, 0),
            FaceWire::East => (1, 0, 0),
            FaceWire::Down => (0, -1, 0),
            FaceWire::Up => (0, 1, 0),
            FaceWire::North => (0, 0, -1),
            FaceWire::South => (0, 0, 1),
        }
    }
}

// ------------------------------------------------------------ server message

/// One block that changed, for batched broadcast.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BlockChange {
    /// Position of the changed block.
    pub pos: BlockPos,
    /// Its new block id.
    pub block: u16,
}

/// All block changes in one server tick, sent as a single message.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Message)]
pub struct BlockChangeBatch {
    /// The changes.
    pub changes: Vec<BlockChange>,
}

/// A chunk's block data, sent when a client needs it.
#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct ChunkData {
    /// Position of the chunk.
    pub pos: (i32, i32, i32),
    /// Palette-compressed chunk bytes (see `Chunk::to_bytes`).
    pub data: Vec<u8>,
}

/// Several chunks in one message, to cut per-message overhead during streaming.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Message)]
pub struct ChunkBatch {
    /// The chunks.
    pub chunks: Vec<ChunkData>,
}

/// A chunk that the client should drop, because it left the interest radius.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Message)]
pub struct ChunkUnload {
    /// Position of the chunk to unload.
    pub pos: (i32, i32, i32),
}

/// The authoritative state of the local player, sent for reconciliation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Message)]
pub struct MoveCorrection {
    /// Sequence number of the last input the server processed.
    pub last_processed_seq: u32,
    /// Authoritative feet position.
    pub position: [f32; 3],
    /// Authoritative velocity.
    pub velocity: [f32; 3],
    /// Whether the player is on the ground.
    pub on_ground: bool,
}

/// A mining operation that the server refused, so the client can stop predicting.
#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct MiningRefused {
    /// The block that was refused.
    pub block: BlockPos,
    /// Why, in words the player can act on.
    pub reason: String,
}

/// Chat and system notifications.
#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct ChatMessage {
    /// Who sent it; empty for server notifications.
    pub from: String,
    /// The text.
    pub text: String,
}

// --------------------------------------------------- replicated components

/// The authoritative transform of a player, replicated to everyone.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct PlayerTransform {
    /// Feet position in world space.
    pub position: [f32; 3],
    /// Yaw in radians.
    pub yaw: f32,
    /// Pitch in radians.
    pub pitch: f32,
}

/// Identity and display data for a player entity.
#[derive(Component, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerInfo {
    /// Server-assigned id.
    pub id: u64,
    /// Display name.
    pub name: String,
}

/// Replicated vitals, visible to the owning player.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Vitals {
    /// Health 0..=100.
    pub health: f32,
    /// Oxygen 0..=100.
    pub oxygen: f32,
    /// Radiation dose, unbounded.
    pub radiation: f32,
}

impl Default for Vitals {
    fn default() -> Self {
        Self {
            health: 100.0,
            oxygen: 100.0,
            radiation: 0.0,
        }
    }
}

/// A mining operation in progress on the server.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct ActiveMining {
    /// The block being mined.
    pub block: BlockPos,
    /// Ticks elapsed.
    pub elapsed: u32,
    /// Ticks required.
    pub required: u32,
}

/// Marker for the entity controlled by a given client connection.
#[derive(Component, Debug, Clone, Copy)]
pub struct OwnedBy {
    /// The client's id.
    pub client_id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello(protocol: u32, hash: u64, name: &str) -> ClientHello {
        ClientHello {
            protocol_version: protocol,
            content_hash: hash,
            display_name: name.to_string(),
        }
    }

    #[test]
    fn matching_handshake_is_accepted() {
        assert_eq!(check_handshake(&hello(2, 123, "miner"), 2, 123), None);
    }

    #[test]
    fn protocol_mismatch_is_refused_with_a_readable_reason() {
        let reason = check_handshake(&hello(1, 123, "miner"), 2, 123).expect("should be refused");
        assert!(reason.contains("Protocol mismatch"), "got {reason:?}");
        assert!(
            reason.contains('1') && reason.contains('2'),
            "should name both versions"
        );
    }

    #[test]
    fn content_mismatch_says_exactly_content_mismatch() {
        let reason = check_handshake(&hello(2, 999, "miner"), 2, 123).expect("should be refused");
        assert_eq!(reason, "Content mismatch");
    }

    #[test]
    fn protocol_is_checked_before_content() {
        // A client with both wrong should hear about the protocol first: that is
        // the more actionable problem.
        let reason = check_handshake(&hello(1, 999, "miner"), 2, 123).expect("refused");
        assert!(reason.contains("Protocol"));
    }

    #[test]
    fn names_are_validated() {
        assert!(check_handshake(&hello(2, 123, "   "), 2, 123).is_some());
        let long = "x".repeat(33);
        assert!(check_handshake(&hello(2, 123, &long), 2, 123).is_some());
        let ok = "x".repeat(32);
        assert_eq!(check_handshake(&hello(2, 123, &ok), 2, 123), None);
    }

    #[test]
    fn face_normals_are_outward() {
        assert_eq!(FaceWire::Up.normal(), (0, 1, 0));
        assert_eq!(FaceWire::North.normal(), (0, 0, -1));
        // Every face normal is a unit axis vector.
        for face in [
            FaceWire::West,
            FaceWire::East,
            FaceWire::Down,
            FaceWire::Up,
            FaceWire::North,
            FaceWire::South,
        ] {
            let (x, y, z) = face.normal();
            assert_eq!(
                x.abs() + y.abs() + z.abs(),
                1,
                "{face:?} is not a unit axis"
            );
        }
    }

    #[test]
    fn protocol_messages_round_trip_through_postcard() {
        // The wire format must survive serialisation, including nested types.
        let hello_msg = ClientHello {
            protocol_version: PROTOCOL_VERSION,
            content_hash: 0xdead_beef_cafe_f00d,
            display_name: "digger".to_string(),
        };
        let bytes = postcard::to_allocvec(&hello_msg).expect("serialize");
        let back: ClientHello = postcard::from_bytes(&bytes).expect("deserialize");
        assert_eq!(back.content_hash, hello_msg.content_hash);
        assert_eq!(back.display_name, "digger");

        let batch = BlockChangeBatch {
            changes: vec![
                BlockChange {
                    pos: BlockPos::new(-5, 300, 12),
                    block: 7,
                },
                BlockChange {
                    pos: BlockPos::new(0, 512, 0),
                    block: 0,
                },
            ],
        };
        let bytes = postcard::to_allocvec(&batch).expect("serialize");
        let back: BlockChangeBatch = postcard::from_bytes(&bytes).expect("deserialize");
        assert_eq!(back.changes.len(), 2);
        assert_eq!(back.changes[0].pos, BlockPos::new(-5, 300, 12));
        assert_eq!(back.changes[1].block, 0);
    }

    #[test]
    fn move_input_round_trips() {
        let input = MoveInput {
            seq: 42,
            dir: [0.5, -0.5],
            jump: true,
            sprint: false,
            crouch: true,
            yaw: 1.25,
            pitch: -0.5,
        };
        let bytes = postcard::to_allocvec(&input).expect("serialize");
        let back: MoveInput = postcard::from_bytes(&bytes).expect("deserialize");
        assert_eq!(back.seq, 42);
        assert!(back.jump);
        assert!(!back.sprint);
        assert!((back.yaw - 1.25).abs() < 1e-6);
    }
}
