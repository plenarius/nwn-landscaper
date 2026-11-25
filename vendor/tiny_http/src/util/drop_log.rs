//! Landscaper patch: the log of connections dropped because the server couldn't take them (no file descriptor left to accept
//! one, no thread free to read it). In such a burst every connection fails the same way, so it logs once when the burst starts,
//! then at most once every [`DropLog::EVERY`] with how many more were dropped, and once when a connection is taken again.

use std::fmt::Display;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::log;

#[derive(Default)]
pub struct DropLog {
    // Set while a burst is on, so a connection taken in the normal run of things doesn't take the lock.
    on: AtomicBool,
    burst: Mutex<Option<Burst>>,
}

struct Burst {
    started: Instant,
    logged: Instant,
    // dropped since the line last logged, and in the whole burst
    since_logged: u64,
    total: u64,
}

impl DropLog {
    /// The longest a burst goes without a line in the log.
    pub const EVERY: Duration = Duration::from_secs(10);

    /// A connection dropped (or not accepted) because of `why`.
    pub fn dropped(&self, why: &dyn Display) {
        let mut burst = self
            .burst
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        match burst.as_mut() {
            None => {
                log::warn!("dropping connections: {why}");
                *burst = Some(Burst {
                    started: now,
                    logged: now,
                    since_logged: 0,
                    total: 1,
                });
                self.on.store(true, Ordering::Release);
            }
            Some(b) => {
                b.since_logged += 1;
                b.total += 1;
                if now.duration_since(b.logged) >= Self::EVERY {
                    log::warn!(
                        "still dropping connections: {} more in {} s ({why})",
                        b.since_logged,
                        now.duration_since(b.logged).as_secs()
                    );
                    b.logged = now;
                    b.since_logged = 0;
                }
            }
        }
    }

    /// A connection taken: ends a burst, if one is on.
    pub fn taken(&self) {
        if !self.on.load(Ordering::Acquire) {
            return;
        }
        let mut burst = self
            .burst
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(b) = burst.take() {
            log::warn!(
                "taking connections again: {} dropped in {} s",
                b.total,
                b.started.elapsed().as_secs()
            );
        }
        self.on.store(false, Ordering::Release);
    }
}
