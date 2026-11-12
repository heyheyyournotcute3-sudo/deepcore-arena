//! Server-side simulation: intent validation, mining timers and world mutation.

pub mod validate;

pub mod mining;
pub mod players;

pub use mining::{MiningTimers, tick_mining};
