//! Regression tests for the Hubcap usage snapshot cache.
//!
//! The problem this cache exists for was visible in a real session log:
//! `Usage stats HTTP 429 Too Many Requests attempt=1/3; retrying` twice, 64 ms
//! apart — the startup badge and the Settings mount asking the same question.
//! `/status/{id}` does not consume the generation quota, but the account is
//! rate-limited, so the answer is fetched once and reused.
//!
//! The rule being tested is deliberately pure (`usage_snapshot_is_fresh`): the
//! tests therefore pin the *policy* (same key, within the TTL) without needing
//! a network or a clock.

use std::time::{Duration, Instant};

use crate::commands::settings::{usage_snapshot_is_fresh, UsageSnapshot};

const TTL: Duration = Duration::from_secs(60);

fn snapshot(api_key: &str, age: Duration, now: Instant) -> UsageSnapshot {
    UsageSnapshot {
        api_key: api_key.to_string(),
        fetched: now - age,
        payload: serde_json::json!({ "usage": 7, "limit": 1500 }),
    }
}

#[test]
fn a_just_fetched_snapshot_is_reused_for_the_same_key() {
    let now = Instant::now();
    let entry = snapshot("key-a", Duration::from_secs(0), now);
    assert!(
        usage_snapshot_is_fresh(&entry, "key-a", now, TTL),
        "the second caller in the same second must reuse the answer (that is the 429 fix)"
    );
}

#[test]
fn a_snapshot_older_than_the_ttl_is_refetched() {
    let now = Instant::now();
    let entry = snapshot("key-a", Duration::from_secs(61), now);
    assert!(!usage_snapshot_is_fresh(&entry, "key-a", now, TTL));

    // Exactly at the TTL the entry is no longer fresh: the window is open
    // below the bound, so "60 s" never stretches to 61.
    let boundary = snapshot("key-a", TTL, now);
    assert!(!usage_snapshot_is_fresh(&boundary, "key-a", now, TTL));
}

#[test]
fn a_different_key_never_reuses_the_previous_account() {
    let now = Instant::now();
    let entry = snapshot("key-a", Duration::from_secs(0), now);
    assert!(
        !usage_snapshot_is_fresh(&entry, "key-b", now, TTL),
        "usage belongs to the account, not to the app: a new key must refetch"
    );
}
