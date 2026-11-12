//! Simulated network conditions for testing.
//!
//! renet 2.0 has no built-in latency or loss simulation, so this module provides
//! a deterministic, clock-driven delay/loss model used by both binaries. It lets
//! us test "does movement still feel responsive at 150 ms?" without a real
//! wide-area link.
//!
//! The model is deliberately simple and *honest*: it delays messages by a fixed
//! one-way latency and drops a percentage of them, which is what a bad
//! connection does. It is not a full network emulator.

use bevy::prelude::*;

/// Network conditions to simulate.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct SimulatedNetwork {
    /// One-way latency in milliseconds.
    pub latency_ms: u32,
    /// Packet loss as a percentage, 0..=100.
    pub loss_percent: u32,
    /// Internal clock, advanced by the caller.
    pub clock_ms: u64,
}

impl SimulatedNetwork {
    /// Whether any simulation is active.
    pub fn is_active(&self) -> bool {
        self.latency_ms > 0 || self.loss_percent > 0
    }

    /// Advances the internal clock.
    pub fn advance(&mut self, delta_ms: u64) {
        self.clock_ms = self.clock_ms.wrapping_add(delta_ms);
    }

    /// Whether a message sent now should be dropped.
    ///
    /// Deterministic in the clock so a test run is reproducible: the same
    /// message sequence always loses the same messages.
    pub fn should_drop(&self, sequence: u64) -> bool {
        if self.loss_percent == 0 {
            return false;
        }
        // xorshift on (sequence, clock) gives a stable pseudo-random decision.
        let mut x = sequence
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(self.clock_ms);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let roll = (x % 100) as u32;
        roll < self.loss_percent
    }

    /// The delivery time for a message sent at the current clock.
    pub fn delivery_time(&self) -> u64 {
        self.clock_ms + self.latency_ms as u64
    }
}

/// A message queued for delayed delivery.
#[derive(Debug, Clone)]
pub struct Delayed<T> {
    /// When the message should be delivered.
    pub deliver_at_ms: u64,
    /// The message.
    pub payload: T,
}

/// A queue that releases items once their delivery time arrives.
#[derive(Debug)]
pub struct DelayQueue<T> {
    /// Queued items, kept sorted by delivery time.
    items: Vec<Delayed<T>>,
    /// Monotonic counter used for the loss decision.
    sequence: u64,
    /// How many items have been dropped, for reporting.
    dropped: u64,
}

impl<T> Default for DelayQueue<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            sequence: 0,
            dropped: 0,
        }
    }
}

impl<T> DelayQueue<T> {
    /// Queues a payload according to the simulated conditions.
    ///
    /// Returns `true` if the payload was accepted (queued or delivered
    /// immediately), `false` if the simulated network dropped it.
    pub fn push(&mut self, payload: T, network: &SimulatedNetwork) -> bool {
        self.sequence += 1;
        if network.should_drop(self.sequence) {
            self.dropped += 1;
            return false;
        }
        self.items.push(Delayed {
            deliver_at_ms: network.delivery_time(),
            payload,
        });
        // Keep sorted so draining is cheap and ordering is stable.
        self.items.sort_by_key(|item| item.deliver_at_ms);
        true
    }

    /// Removes and returns everything due at or before `now_ms`.
    pub fn drain_ready(&mut self, now_ms: u64) -> Vec<T> {
        let mut ready = Vec::new();
        let mut remaining = Vec::new();
        for item in self.items.drain(..) {
            if item.deliver_at_ms <= now_ms {
                ready.push(item.payload);
            } else {
                remaining.push(item);
            }
        }
        self.items = remaining;
        ready
    }

    /// Number of items waiting.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of items the simulated network has dropped.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// Parses the two simulation flags shared by both binaries.
///
/// Kept in `shared` so the client and server cannot disagree about the meaning
/// of the flags.
pub fn parse_simulation_flags(args: &[String]) -> (u32, u32) {
    let mut latency = 0;
    let mut loss = 0;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--simulate-latency" => {
                if let Some(value) = args.get(index + 1).and_then(|v| v.parse::<u32>().ok()) {
                    latency = value;
                }
                index += 2;
            }
            "--simulate-loss" => {
                if let Some(value) = args.get(index + 1).and_then(|v| v.parse::<u32>().ok()) {
                    loss = value.min(100);
                }
                index += 2;
            }
            _ => index += 1,
        }
    }
    (latency, loss)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_simulation_means_no_delay_and_no_loss() {
        let network = SimulatedNetwork::default();
        assert!(!network.is_active());
        assert!(!network.should_drop(1));
        let mut queue: DelayQueue<u32> = DelayQueue::default();
        assert!(queue.push(1, &network));
        assert_eq!(
            queue.drain_ready(0),
            vec![1],
            "no latency means immediate delivery"
        );
    }

    #[test]
    fn latency_delays_delivery_by_exactly_that_much() {
        let network = SimulatedNetwork {
            latency_ms: 150,
            ..default()
        };
        let mut queue: DelayQueue<u32> = DelayQueue::default();
        queue.push(42, &network);
        // Not yet due.
        assert!(queue.drain_ready(0).is_empty());
        assert!(queue.drain_ready(149).is_empty());
        // Due at exactly 150.
        assert_eq!(queue.drain_ready(150), vec![42]);
    }

    #[test]
    fn loss_drops_a_proportion_of_messages() {
        let network = SimulatedNetwork {
            loss_percent: 50,
            ..default()
        };
        let mut queue: DelayQueue<u32> = DelayQueue::default();
        let mut accepted = 0;
        for i in 0..1000 {
            if queue.push(i, &network) {
                accepted += 1;
            }
        }
        // Around half should survive. Allow a wide band because the hash is not
        // a true uniform generator.
        assert!(
            (300..700).contains(&accepted),
            "expected roughly 50% delivered, got {accepted}"
        );
        assert_eq!(queue.dropped() as usize, 1000 - accepted);
    }

    #[test]
    fn total_loss_drops_everything() {
        let network = SimulatedNetwork {
            loss_percent: 100,
            ..default()
        };
        let mut queue: DelayQueue<u32> = DelayQueue::default();
        for i in 0..100 {
            assert!(!queue.push(i, &network), "100% loss must drop everything");
        }
        assert!(queue.is_empty());
    }

    #[test]
    fn loss_is_deterministic_for_the_same_inputs() {
        let network = SimulatedNetwork {
            loss_percent: 30,
            latency_ms: 80,
            ..default()
        };
        let run = || {
            let mut queue: DelayQueue<u32> = DelayQueue::default();
            for i in 0..500 {
                queue.push(i, &network);
            }
            queue.dropped()
        };
        assert_eq!(run(), run(), "the same inputs must drop the same messages");
    }

    #[test]
    fn messages_are_delivered_in_order() {
        let network = SimulatedNetwork {
            latency_ms: 100,
            ..default()
        };
        let mut queue: DelayQueue<u32> = DelayQueue::default();
        for i in 0..10 {
            queue.push(i, &network);
        }
        let delivered = queue.drain_ready(100);
        assert_eq!(delivered, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn flags_parse_and_clamp() {
        let args: Vec<String> = ["--simulate-latency", "150", "--simulate-loss", "5"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(parse_simulation_flags(&args), (150, 5));

        let args: Vec<String> = ["--simulate-loss", "500"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            parse_simulation_flags(&args),
            (0, 100),
            "loss is clamped to 100"
        );

        assert_eq!(parse_simulation_flags(&[]), (0, 0));
    }
}
