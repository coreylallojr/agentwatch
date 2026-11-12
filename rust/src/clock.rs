//! `Date.now()`, with an override used by the parity harness (`AGENTWATCH_FAKE_NOW`, read only by `__dump`).
use std::sync::atomic::{AtomicU64, Ordering};

static FAKE: AtomicU64 = AtomicU64::new(0);

pub fn set_fake_now(ms: f64) { FAKE.store(ms.to_bits(), Ordering::Relaxed); }

pub fn now_ms() -> f64 {
    let f = FAKE.load(Ordering::Relaxed);
    if f != 0 { return f64::from_bits(f); }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}
