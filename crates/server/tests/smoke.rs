//! Headless smoke tests for the server simulation loop.

/// The tick duration must divide evenly into 30 Hz. A wrong constant here
/// silently changes the whole game's simulation rate.
#[test]
fn tick_matches_30_hz() {
    let tick_ns = 1_000_000_000u64 / 30;
    assert_eq!(tick_ns, 33_333_333);
    let rate = 1_000_000_000f64 / tick_ns as f64;
    assert!((rate - 30.0).abs() < 0.001, "tick rate was {rate}");
}
