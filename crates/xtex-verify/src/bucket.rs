//! Politeness, mechanically: a fixed token bucket per source.
//!
//! The limits are documented by the sources themselves (the Crossref
//! polite pool, `OpenAlex` with `mailto`), so nothing adaptive is needed — the bucket
//! simply refuses to go faster than the etiquette allows.

use std::time::{Duration, Instant};

/// A minimum-interval gate: at most one pass per `interval`.
pub struct Bucket {
    interval: Duration,
    last: Option<Instant>,
}

impl Bucket {
    /// A bucket allowing one pass per `interval`.
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    /// Blocks until a pass is allowed, then takes it.
    pub fn take(&mut self) {
        if let Some(last) = self.last {
            let elapsed = last.elapsed();
            if elapsed < self.interval {
                std::thread::sleep(self.interval.saturating_sub(elapsed));
            }
        }
        self.last = Some(Instant::now());
    }
}
