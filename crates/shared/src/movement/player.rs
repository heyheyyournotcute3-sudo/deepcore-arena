//! The player controller: input, physics constants and one deterministic step.
//!
//! [`step_player`] is the single source of truth for how a player moves. The
//! client calls it every frame to predict; the server calls it every fixed tick
//! to decide. Because it is pure, a mismatch can only come from a difference in
//! input, never from a difference in code.

use bevy::math::Vec3;

use super::collide::{Aabb, BlockSolid, sweep_aabb};
use super::{EYE_HEIGHT, EYE_HEIGHT_CROUCH, PLAYER_HEIGHT, PLAYER_HEIGHT_CROUCH, PLAYER_WIDTH};

/// A single frame of player input.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerInput {
    /// Movement sequence number, used for reconciliation.
    pub seq: u32,
    /// Desired movement direction in local space: x = right, z = forward.
    pub dir: [f32; 2],
    /// Whether the jump key is held.
    pub jump: bool,
    /// Whether sprint is held.
    pub sprint: bool,
    /// Whether crouch is held.
    pub crouch: bool,
    /// Look yaw in radians.
    pub yaw: f32,
    /// Look pitch in radians.
    pub pitch: f32,
}

/// Tunable physics constants for the player controller.
#[derive(Clone, Copy, Debug)]
pub struct PlayerPhysics {
    /// Downward acceleration in blocks/s².
    pub gravity: f32,
    /// Upward speed applied on jump.
    pub jump_speed: f32,
    /// Walking speed in blocks/s.
    pub walk_speed: f32,
    /// Sprinting speed in blocks/s.
    pub sprint_speed: f32,
    /// Crouching speed in blocks/s.
    pub crouch_speed: f32,
    /// How quickly horizontal velocity approaches the target, per second.
    pub ground_accel: f32,
    /// Air control acceleration, per second.
    pub air_accel: f32,
    /// Terminal fall speed.
    pub max_fall_speed: f32,
    /// Largest movement allowed in one step, in blocks.
    pub max_step: f32,
}

impl Default for PlayerPhysics {
    fn default() -> Self {
        Self {
            gravity: 26.0,
            jump_speed: 8.2,
            walk_speed: 4.4,
            sprint_speed: 6.8,
            crouch_speed: 2.0,
            ground_accel: 14.0,
            air_accel: 3.0,
            max_fall_speed: 60.0,
            // Below one block so collision can never tunnel.
            max_step: 0.45,
        }
    }
}

/// The mutable physical state of a player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    /// Feet position.
    pub position: Vec3,
    /// Current velocity.
    pub velocity: Vec3,
    /// Whether the player is on the ground.
    pub on_ground: bool,
    /// Whether the player is currently crouching (affects hitbox height).
    pub crouching: bool,
    /// Yaw in radians.
    pub yaw: f32,
    /// Pitch in radians, clamped to straight up/down.
    pub pitch: f32,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.5, 512.0, 0.5),
            velocity: Vec3::ZERO,
            on_ground: false,
            crouching: false,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

impl PlayerState {
    /// The player's collision box.
    pub fn aabb(&self) -> Aabb {
        let height = if self.crouching {
            PLAYER_HEIGHT_CROUCH
        } else {
            PLAYER_HEIGHT
        };
        Aabb::from_feet(self.position, PLAYER_WIDTH, height)
    }

    /// Eye position, where the camera and the mining ray start.
    pub fn eye_position(&self) -> Vec3 {
        let eye = if self.crouching {
            EYE_HEIGHT_CROUCH
        } else {
            EYE_HEIGHT
        };
        self.position + Vec3::Y * eye
    }

    /// The unit vector the player is looking along.
    pub fn look_direction(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch).normalize_or_zero()
    }
}

/// Advances the player by one fixed timestep.
///
/// Pure and deterministic: no randomness, no wall-clock reads.
pub fn step_player(
    state: &mut PlayerState,
    input: &PlayerInput,
    physics: &PlayerPhysics,
    dt: f32,
    world: &impl BlockSolid,
) -> StepOutcome {
    // --- look ------------------------------------------------------------
    state.yaw = input.yaw;
    // Clamp pitch just shy of vertical to keep the look vector well defined.
    let limit = std::f32::consts::FRAC_PI_2 - 0.001;
    state.pitch = input.pitch.clamp(-limit, limit);

    // --- crouch ----------------------------------------------------------
    state.crouching = input.crouch && state.on_ground;

    // --- desired horizontal velocity -------------------------------------
    let mut wish = Vec3::new(input.dir[0], 0.0, input.dir[1]);
    if wish.length_squared() > 1.0 {
        wish = wish.normalize();
    }
    // Rotate the local wish direction by yaw into world space.
    let (sin_yaw, cos_yaw) = state.yaw.sin_cos();
    let world_wish = Vec3::new(
        wish.x * cos_yaw - wish.z * sin_yaw,
        0.0,
        -wish.x * sin_yaw - wish.z * cos_yaw,
    );

    let speed = if state.crouching {
        physics.crouch_speed
    } else if input.sprint {
        physics.sprint_speed
    } else {
        physics.walk_speed
    };
    let target = world_wish * speed;

    let accel = if state.on_ground {
        physics.ground_accel
    } else {
        physics.air_accel
    };
    // Exponential approach, framerate independent.
    let blend = 1.0 - (-accel * dt).exp();
    state.velocity.x += (target.x - state.velocity.x) * blend;
    state.velocity.z += (target.z - state.velocity.z) * blend;

    // --- jumping ---------------------------------------------------------
    let mut jumped = false;
    if input.jump && state.on_ground {
        state.velocity.y = physics.jump_speed;
        state.on_ground = false;
        jumped = true;
    }

    // --- gravity ---------------------------------------------------------
    state.velocity.y -= physics.gravity * dt;
    if state.velocity.y < -physics.max_fall_speed {
        state.velocity.y = -physics.max_fall_speed;
    }

    // --- integrate with substeps -----------------------------------------
    let delta = state.velocity * dt;
    // Cap per-substep travel so collision cannot tunnel.
    let max_component = delta.abs().max_element();
    let substeps = ((max_component / physics.max_step).ceil() as usize).clamp(1, 32);
    let step_delta = delta / substeps as f32;

    let mut on_ground = false;
    let mut fell_distance = 0.0f32;

    for _ in 0..substeps {
        let start = state.aabb();
        let result = sweep_aabb(start, step_delta, world);
        state.position += result.translation;

        if result.hit_y {
            if step_delta.y < 0.0 {
                on_ground = true;
            }
            state.velocity.y = 0.0;
        }
        if result.hit_x {
            state.velocity.x = 0.0;
        }
        if result.hit_z {
            state.velocity.z = 0.0;
        }
        // Track only the blocked portion of the fall, for fall damage.
        if step_delta.y < 0.0 {
            fell_distance += step_delta.y - result.translation.y;
        }
    }

    state.on_ground = on_ground;

    StepOutcome {
        jumped,
        landed: on_ground,
        fell_distance: fell_distance.abs(),
    }
}

/// Side effects of one step, for the caller to react to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepOutcome {
    /// Whether a jump was started this step.
    pub jumped: bool,
    /// Whether the player is touching ground after the step.
    pub landed: bool,
    /// How far the player fell before being stopped, in blocks.
    pub fell_distance: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptyWorld;
    impl BlockSolid for EmptyWorld {
        fn is_solid(&self, _x: i32, _y: i32, _z: i32) -> bool {
            false
        }
    }

    struct FloorAt(i32);
    impl BlockSolid for FloorAt {
        fn is_solid(&self, _x: i32, y: i32, _z: i32) -> bool {
            y <= self.0
        }
    }

    fn input_forward(sprint: bool) -> PlayerInput {
        PlayerInput {
            dir: [0.0, 1.0],
            sprint,
            ..Default::default()
        }
    }

    #[test]
    fn walking_moves_forward_along_yaw() {
        let world = EmptyWorld;
        let physics = PlayerPhysics::default();
        let mut state = PlayerState::default();
        let input = input_forward(false);
        for _ in 0..10 {
            step_player(&mut state, &input, &physics, 1.0 / 30.0, &world);
        }
        // With yaw 0, forward is -Z, and there is no sideways drift.
        assert!(
            state.position.z < 0.5,
            "did not move forward: {:?}",
            state.position
        );
        assert!(
            (state.position.x - 0.5).abs() < 1e-3,
            "unexpected sideways drift: {}",
            state.position.x
        );
    }

    #[test]
    fn gravity_pulls_the_player_down_and_floor_stops_them() {
        let world = FloorAt(510);
        let physics = PlayerPhysics::default();
        let mut state = PlayerState {
            position: Vec3::new(0.5, 514.0, 0.5),
            ..Default::default()
        };
        let idle = PlayerInput::default();
        for _ in 0..200 {
            step_player(&mut state, &idle, &physics, 1.0 / 30.0, &world);
        }
        assert!(state.on_ground, "player never landed");
        // Floor top is y=511, so feet rest at 511.
        assert!(
            (state.position.y - 511.0).abs() < 0.1,
            "resting height was {}",
            state.position.y
        );
    }

    #[test]
    fn jump_leaves_the_ground() {
        let world = FloorAt(510);
        let physics = PlayerPhysics::default();
        let mut state = PlayerState {
            position: Vec3::new(0.5, 511.0, 0.5),
            on_ground: true,
            ..Default::default()
        };
        let jump = PlayerInput {
            jump: true,
            ..Default::default()
        };
        let outcome = step_player(&mut state, &jump, &physics, 1.0 / 30.0, &world);
        assert!(outcome.jumped);
        assert!(state.position.y > 511.0, "did not rise");
    }

    #[test]
    fn sprint_is_faster_than_walking() {
        let world = EmptyWorld;
        let physics = PlayerPhysics::default();
        let mut walker = PlayerState::default();
        let mut sprinter = PlayerState::default();
        for _ in 0..30 {
            step_player(
                &mut walker,
                &input_forward(false),
                &physics,
                1.0 / 30.0,
                &world,
            );
            step_player(
                &mut sprinter,
                &input_forward(true),
                &physics,
                1.0 / 30.0,
                &world,
            );
        }
        let walk_dist = (walker.position.z - 0.5).abs();
        let sprint_dist = (sprinter.position.z - 0.5).abs();
        assert!(
            sprint_dist > walk_dist * 1.2,
            "sprint {sprint_dist} not meaningfully faster than walk {walk_dist}"
        );
    }

    #[test]
    fn movement_is_deterministic_across_runs() {
        let physics = PlayerPhysics::default();
        let run = || {
            let world = FloorAt(500);
            let mut state = PlayerState {
                position: Vec3::new(3.25, 511.0, -2.5),
                ..Default::default()
            };
            let mut input = PlayerInput {
                dir: [0.6, 0.8],
                sprint: true,
                ..Default::default()
            };
            for i in 0..120 {
                input.yaw = i as f32 * 0.01;
                input.seq = i;
                step_player(&mut state, &input, &physics, 1.0 / 30.0, &world);
            }
            state.position
        };
        let a = run();
        let b = run();
        assert_eq!(a, b, "identical inputs must produce identical positions");
    }

    #[test]
    fn pitch_is_clamped_short_of_vertical() {
        let world = EmptyWorld;
        let physics = PlayerPhysics::default();
        let mut state = PlayerState::default();
        let input = PlayerInput {
            pitch: 10.0,
            ..Default::default()
        };
        step_player(&mut state, &input, &physics, 1.0 / 30.0, &world);
        assert!(state.pitch < std::f32::consts::FRAC_PI_2);
        assert!(state.look_direction().is_normalized());
    }

    #[test]
    fn look_direction_points_where_yaw_says() {
        let state = PlayerState {
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        let dir = state.look_direction();
        assert!(
            (dir.z + 1.0).abs() < 1e-3,
            "yaw 0 should look down -Z, got {dir:?}"
        );
    }

    #[test]
    fn player_cannot_walk_through_a_wall() {
        // A floor at y=510 plus a wall spanning the body height, so the player
        // stays at the wall's height instead of falling past it.
        struct WallAt(i32);
        impl BlockSolid for WallAt {
            fn is_solid(&self, _x: i32, y: i32, z: i32) -> bool {
                if y <= 510 {
                    return true;
                }
                z == self.0 && (511..=513).contains(&y)
            }
        }
        let world = WallAt(-1);
        let physics = PlayerPhysics::default();
        let mut state = PlayerState {
            position: Vec3::new(0.5, 511.0, 0.5),
            on_ground: true,
            ..Default::default()
        };
        let input = input_forward(false);
        for _ in 0..120 {
            step_player(&mut state, &input, &physics, 1.0 / 30.0, &world);
        }
        // Wall occupies z = -1..0, player half depth 0.3, so feet stop near -0.3.
        assert!(
            state.position.z > -0.4,
            "walked through the wall to {}",
            state.position.z
        );
    }
}
