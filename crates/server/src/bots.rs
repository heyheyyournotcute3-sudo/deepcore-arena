//! Headless bot clients for load and desync testing.
//!
//! Each bot is a real player with its own movement state, driven by a
//! deterministic pseudo-random walk, and it goes through the *same* intent
//! validation as a remote client. Bots exist to prove the server stays correct
//! under churn: eight of them moving and digging for ten minutes must not panic,
//! desync, or leak entities.
//!
//! Bots run inside the server process rather than over sockets, so a soak test
//! measures simulation cost without eight real connections. `--simulate-latency`
//! and `--simulate-loss` are exercised separately by the real client.

use bevy::prelude::*;

use deepcore_shared::content::ContentRegistry;
use deepcore_shared::movement::{PlayerInput, PlayerPhysics, PlayerState, step_player};
use deepcore_shared::protocol::FaceWire;
use deepcore_shared::voxel::{BlockId, BlockPos};
use deepcore_shared::worldgen::WorldGen;

use crate::TickCounter;
use crate::sim::mining::MiningTimers;
use crate::sim::validate::{PlayerView, validate_mining};
use crate::world::ServerWorld;

/// One bot's state.
#[derive(Debug)]
pub struct Bot {
    /// Bot id, used in place of a client id.
    pub id: u64,
    /// Physics state.
    pub state: PlayerState,
    /// Deterministic RNG state, so a soak run is reproducible.
    rng: u64,
    /// Current movement intent.
    input: PlayerInput,
    /// Ticks until the intent is re-rolled.
    input_timer: u32,
    /// Mining attempts the validator accepted.
    pub accepted: u32,
    /// Mining attempts the validator rejected.
    pub rejected: u32,
}

impl Bot {
    /// Creates a bot with a seeded RNG.
    pub fn new(id: u64, spawn: PlayerState) -> Self {
        Self {
            id,
            state: spawn,
            rng: 0x9E37_79B9_7F4A_7C15 ^ id.wrapping_mul(0x2545_F491_4F6C_DD1D),
            input: PlayerInput::default(),
            input_timer: 0,
            accepted: 0,
            rejected: 0,
        }
    }

    /// Next pseudo-random float in 0..1 (xorshift64).
    fn next_f32(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Picks a new random movement intent.
    fn reroll(&mut self) {
        self.input = PlayerInput {
            seq: 0,
            dir: [self.next_f32() * 2.0 - 1.0, self.next_f32() * 2.0 - 1.0],
            jump: self.next_f32() > 0.85,
            sprint: self.next_f32() > 0.7,
            crouch: false,
            yaw: self.next_f32() * std::f32::consts::TAU,
            // Look downward so bots aim at the ground and actually mine.
            pitch: -1.2,
        };
        self.input_timer = 15 + (self.next_f32() * 60.0) as u32;
    }

    /// The player view the validator checks.
    fn view(&self) -> PlayerView {
        let aabb = self.state.aabb();
        PlayerView {
            eye: self.state.eye_position(),
            look: self.state.look_direction(),
            tool: deepcore_shared::mining::MiningTool::HANDS,
            hotbar: [None; 9],
            body_min: aabb.min(),
            body_max: aabb.max(),
        }
    }
}

/// Every bot in the session.
#[derive(Resource, Default)]
pub struct Bots {
    /// The bots.
    pub bots: Vec<Bot>,
}

/// The bot plugin.
pub struct BotPlugin {
    count: usize,
}

impl BotPlugin {
    /// Creates the plugin with a bot count.
    pub fn new(count: usize) -> Self {
        Self { count }
    }
}

impl Plugin for BotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Bots::default())
            .insert_resource(BotCount(self.count))
            .add_systems(FixedUpdate, (tick_bots, report_bots).chain());
    }
}

/// Startup parameter for the bot count.
#[derive(Resource)]
struct BotCount(usize);

/// Advances every bot: move, then occasionally try to mine.
fn tick_bots(
    mut bots: ResMut<Bots>,
    bot_count: Res<BotCount>,
    mut world: ResMut<ServerWorld>,
    mut timers: ResMut<MiningTimers>,
    world_gen: Res<WorldGen>,
    registry: Res<ContentRegistry>,
    physics: Option<Res<PlayerPhysics>>,
) {
    // Spawn on the first tick, once resources exist.
    if bots.bots.is_empty() && bot_count.0 > 0 {
        let surface = WorldGen::surface_y() as f32;
        for index in 0..bot_count.0 {
            let angle = index as f32 * 0.7;
            let radius = 3.0 + index as f32 * 1.5;
            let spawn = PlayerState {
                position: Vec3::new(
                    0.5 + angle.cos() * radius,
                    surface + 1.0,
                    0.5 + angle.sin() * radius,
                ),
                ..default()
            };
            bots.bots.push(Bot::new(1000 + index as u64, spawn));
        }
        info!(count = bots.bots.len(), "bots spawned");
    }

    if bots.bots.is_empty() {
        return;
    }

    // Make sure the ground exists before bots walk on it.
    let center = deepcore_shared::voxel::ChunkPos::new(0, 16, 0);
    crate::world::ensure_chunks_around(&mut world, &world_gen, center, 2, 1);

    let physics = physics.map(|p| *p).unwrap_or_default();
    let blocks = BotWorld { world: &world };

    for bot in bots.bots.iter_mut() {
        if bot.input_timer == 0 {
            bot.reroll();
        } else {
            bot.input_timer -= 1;
        }
        step_player(&mut bot.state, &bot.input, &physics, 1.0 / 30.0, &blocks);

        // Try to mine every 20 ticks.
        if bot.input_timer % 20 == 0 {
            let eye = bot.state.eye_position();
            let look = bot.state.look_direction();
            let target = BlockPos::new(
                (eye.x + look.x * 2.0).floor() as i32,
                (eye.y + look.y * 2.0).floor() as i32,
                (eye.z + look.z * 2.0).floor() as i32,
            );
            let view = bot.view();
            match validate_mining(&view, target, FaceWire::Up, &registry, &blocks) {
                Ok(plan) => {
                    timers.start(bot.id, plan.block, plan.ticks);
                    bot.accepted += 1;
                }
                Err(_) => bot.rejected += 1,
            }
        }
    }
}

/// Logs bot progress once per second for the soak test.
fn report_bots(bots: Res<Bots>, ticks: Res<TickCounter>, mut last: Local<u64>) {
    if bots.bots.is_empty() || ticks.ticks == 0 {
        return;
    }
    if ticks.ticks % 30 != 0 || ticks.ticks == *last {
        return;
    }
    *last = ticks.ticks;
    let accepted: u32 = bots.bots.iter().map(|b| b.accepted).sum();
    let rejected: u32 = bots.bots.iter().map(|b| b.rejected).sum();
    let avg_y: f32 =
        bots.bots.iter().map(|b| b.state.position.y).sum::<f32>() / bots.bots.len() as f32;
    info!(
        bots = bots.bots.len(),
        accepted,
        rejected,
        avg_y = format_args!("{avg_y:.1}"),
        ticks = ticks.ticks,
        "bot soak"
    );
}

/// Block access for bots: both collision (`BlockSolid`) and intent validation
/// (`BlockSource`), so a bot walks and mines against the same authoritative world
/// a remote client does.
struct BotWorld<'a> {
    world: &'a ServerWorld,
}

impl deepcore_shared::movement::BlockSolid for BotWorld<'_> {
    fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        match self.world.block_at(BlockPos::new(x, y, z)) {
            Some(block) => !block.is_air(),
            // Unloaded space is solid so bots cannot walk out of the world.
            None => true,
        }
    }
}

impl crate::sim::validate::BlockSource for BotWorld<'_> {
    fn block(&self, pos: BlockPos) -> Option<BlockId> {
        self.world.block_at(pos)
    }
}
