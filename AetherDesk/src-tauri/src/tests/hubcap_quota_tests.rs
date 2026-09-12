use std::time::{Duration, SystemTime};

use crate::providers::hubcap_generation::{
    current_est_day, release_quota_at, reserve_quota_at, GenerationKind,
    MAX_GAME_GENERATIONS_PER_DAY, MAX_WORKSHOP_GENERATIONS_PER_DAY,
};

/// Isolated quota file (and sibling lock) in a temp directory.
fn temp_quota_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "aether_quota_tests_{}_{}",
        std::process::id(),
        tag
    ))
}

fn write_quota_fixture(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).expect("quota fixture write");
}

fn read_game_used(path: &std::path::Path) -> u64 {
    let bytes = std::fs::read(path).expect("quota file read");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("quota json parse");
    value["game_used"].as_u64().expect("game_used field")
}

#[test]
fn reserve_increments_and_enforces_the_shared_limit() {
    let path = temp_quota_path("limit");
    let _ = std::fs::remove_file(&path);
    let lock = path.with_extension("lock");
    let _ = std::fs::remove_file(&lock);

    reserve_quota_at(&path, GenerationKind::Game).expect("first reserve succeeds");
    assert_eq!(read_game_used(&path), 1, "reserve must persist the increment");

    // Pre-fill the budget to the limit: the next reserve is rejected and the
    // counter must stay pinned at the limit.
    write_quota_fixture(
        &path,
        &format!(
            "{{\"day\":\"{}\",\"game_used\":{},\"workshop_used\":0}}",
            current_est_day(),
            MAX_GAME_GENERATIONS_PER_DAY
        ),
    );
    let denied = reserve_quota_at(&path, GenerationKind::Game)
        .expect_err("budget at the limit must be rejected");
    assert!(denied.contains("limit reached"), "unexpected error: {denied}");
    assert_eq!(
        read_game_used(&path),
        u64::from(MAX_GAME_GENERATIONS_PER_DAY),
        "a rejected reserve must not change the counters"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&lock);
}

#[test]
fn release_returns_the_reserved_unit() {
    let path = temp_quota_path("release");
    let _ = std::fs::remove_file(&path);
    let lock = path.with_extension("lock");
    let _ = std::fs::remove_file(&lock);

    reserve_quota_at(&path, GenerationKind::Game).expect("reserve");
    release_quota_at(&path, GenerationKind::Game);
    assert_eq!(read_game_used(&path), 0, "release must give the unit back");
    // Releasing with nothing reserved must not underflow.
    release_quota_at(&path, GenerationKind::Game);
    assert_eq!(read_game_used(&path), 0, "release is saturating");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&lock);
}

#[test]
fn day_rollover_resets_the_counters() {
    let path = temp_quota_path("rollover");
    let _ = std::fs::remove_file(&path);
    let lock = path.with_extension("lock");
    let _ = std::fs::remove_file(&lock);

    // Yesterday's budget was exhausted: today's must start fresh.
    write_quota_fixture(
        &path,
        &format!(
            "{{\"day\":\"2000-01-01\",\"game_used\":{},\"workshop_used\":{}}}",
            MAX_GAME_GENERATIONS_PER_DAY, MAX_WORKSHOP_GENERATIONS_PER_DAY
        ),
    );
    reserve_quota_at(&path, GenerationKind::Game)
        .expect("a new fixed-EST day resets the budget");
    reserve_quota_at(&path, GenerationKind::Workshop)
        .expect("the Workshop budget resets with the same day");
    assert_eq!(read_game_used(&path), 1);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&lock);
}

#[test]
fn corrupt_quota_file_is_treated_as_fresh() {
    let path = temp_quota_path("corrupt");
    let _ = std::fs::remove_file(&path);
    let lock = path.with_extension("lock");
    let _ = std::fs::remove_file(&lock);

    write_quota_fixture(&path, "not json at all");
    reserve_quota_at(&path, GenerationKind::Game)
        .expect("a corrupt file must never wedge generation");
    assert_eq!(read_game_used(&path), 1);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&lock);
}

#[test]
fn stale_lock_is_broken_and_cleaned_up() {
    let path = temp_quota_path("stale_lock");
    let _ = std::fs::remove_file(&path);
    let lock = path.with_extension("lock");
    let _ = std::fs::remove_file(&lock);

    // A leftover lock from a crashed holder: older than the stale threshold.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .expect("lock fixture create");
    file.set_modified(SystemTime::now() - Duration::from_secs(60))
        .expect("lock fixture mtime");
    drop(file);

    reserve_quota_at(&path, GenerationKind::Game)
        .expect("a stale lock must be broken, not obeyed");
    assert_eq!(read_game_used(&path), 1);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&lock);
}

#[test]
fn budgets_are_independent_per_kind() {
    let path = temp_quota_path("kinds");
    let _ = std::fs::remove_file(&path);
    let lock = path.with_extension("lock");
    let _ = std::fs::remove_file(&lock);

    write_quota_fixture(
        &path,
        &format!(
            "{{\"day\":\"{}\",\"game_used\":{},\"workshop_used\":0}}",
            current_est_day(),
            MAX_GAME_GENERATIONS_PER_DAY
        ),
    );
    let denied = reserve_quota_at(&path, GenerationKind::Game).expect_err("game budget is spent");
    assert!(denied.contains("game-manifest"), "unexpected error: {denied}");
    reserve_quota_at(&path, GenerationKind::Workshop)
        .expect("the Workshop budget is independent");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&lock);
}
