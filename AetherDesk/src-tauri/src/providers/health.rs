//! Process-wide reachability of the remote sources the app already talks to.
//!
//! Nothing here probes the network: callers record the outcome of a request
//! they were making anyway. Other error paths read the latest outcome to tell
//! "the host did not answer" from "the host answered with a failure", which is
//! what separates an ISP/DNS block from an ordinary service error.

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Sources whose reachability the store and its downloads depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreSource {
    Steam,
    Hubcap,
}

#[derive(Debug, Clone, Copy)]
struct Outcome {
    reachable: bool,
    at: Instant,
}

/// One slot per source, indexed by [`slot_index`].
fn outcomes() -> &'static Mutex<[Option<Outcome>; 2]> {
    static OUTCOMES: OnceLock<Mutex<[Option<Outcome>; 2]>> = OnceLock::new();
    OUTCOMES.get_or_init(|| Mutex::new([None, None]))
}

fn slot_index(source: StoreSource) -> usize {
    match source {
        StoreSource::Steam => 0,
        StoreSource::Hubcap => 1,
    }
}

fn lock_outcomes() -> MutexGuard<'static, [Option<Outcome>; 2]> {
    // A poisoned lock only means another thread panicked while recording; the
    // slots hold plain `Copy` data, so recovering them is safe.
    outcomes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Record whether `source` answered the latest request.
///
/// An `Ok` response with zero payload still counts as an answer: the host
/// resolved, accepted the request and replied.
pub fn record(source: StoreSource, reachable: bool) {
    record_at(source, reachable, Instant::now());
}

/// Testable variant of [`record`]: the caller supplies the observation time.
fn record_at(source: StoreSource, reachable: bool, at: Instant) {
    lock_outcomes()[slot_index(source)] = Some(Outcome { reachable, at });
}

/// True when `source` answered within `max_age` of now.
pub fn answered_recently(source: StoreSource, max_age: Duration) -> bool {
    answered_recently_at(source, max_age, Instant::now())
}

/// Testable variant of [`answered_recently`]: the caller supplies "now", so
/// the TTL rule is checked without sleeping.
fn answered_recently_at(source: StoreSource, max_age: Duration, now: Instant) -> bool {
    lock_outcomes()[slot_index(source)].is_some_and(|outcome| {
        outcome.reachable && now.saturating_duration_since(outcome.at) <= max_age
    })
}

#[cfg(test)]
mod tests {
    use super::{answered_recently_at, record_at, StoreSource};
    use std::time::{Duration, Instant};

    #[test]
    fn steam_answer_is_fresh_until_the_ttl_expires() {
        let observed = Instant::now();
        record_at(StoreSource::Steam, true, observed);
        assert!(answered_recently_at(
            StoreSource::Steam,
            Duration::from_secs(300),
            observed + Duration::from_secs(299),
        ));
        assert!(answered_recently_at(
            StoreSource::Steam,
            Duration::from_secs(300),
            observed + Duration::from_secs(300),
        ));
        assert!(!answered_recently_at(
            StoreSource::Steam,
            Duration::from_secs(300),
            observed + Duration::from_secs(301),
        ));
    }

    #[test]
    fn an_unreachable_outcome_never_counts_as_an_answer() {
        let observed = Instant::now();
        record_at(StoreSource::Hubcap, false, observed);
        assert!(!answered_recently_at(
            StoreSource::Hubcap,
            Duration::from_secs(300),
            observed,
        ));

        record_at(StoreSource::Hubcap, true, observed);
        assert!(answered_recently_at(
            StoreSource::Hubcap,
            Duration::from_secs(300),
            observed,
        ));
    }
}
