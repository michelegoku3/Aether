use crate::manifest::pins::{pins_from_rows, DepotManifestPin, LuaManifestRow};
use crate::manifest::resolver::{manifest_file_name, restore_local_blocking, verify_available};

fn pin(depot: u32, gid: &str) -> DepotManifestPin {
    DepotManifestPin {
        depot_id: depot,
        manifest_id: gid.to_string(),
    }
}

fn row(depot: u32, gid: &str, enabled: bool) -> LuaManifestRow {
    LuaManifestRow {
        row_id: 0,
        app_id: depot,
        manifest_id: gid.to_string(),
        enabled,
        issue: None,
    }
}

/// Isolated fake Steam root with an existing `depotcache`. The app_id used in
/// these tests never matches a real AetherData backup folder, so the backup
/// search dir is guaranteed to be absent.
fn temp_steam_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("aether_resolver_tests_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("depotcache")).expect("temp depotcache");
    dir
}

#[test]
fn manifest_file_name_matches_steam_identity() {
    assert_eq!(
        manifest_file_name(&pin(3618390, "8472815076543210")),
        "3618390_8472815076543210.manifest"
    );
}

#[test]
fn pins_from_rows_maps_rows_without_filtering() {
    let pins = pins_from_rows([row(1, "11", true), row(2, "22", false)]);
    assert_eq!(pins.len(), 2, "filtering is the caller's policy");
    assert_eq!(pins[0].depot_id, 1);
    assert_eq!(pins[0].manifest_id, "11");
    assert_eq!(pins[1].depot_id, 2);
    assert_eq!(pins[1].manifest_id, "22");
}

#[test]
fn verify_available_counts_verified_and_missing_pins() {
    let steam = temp_steam_dir("verify");
    let present = pin(111, "1001");
    std::fs::write(
        steam.join("depotcache").join(manifest_file_name(&present)),
        b"payload",
    )
    .expect("write manifest");
    let empty = pin(333, "3003");
    std::fs::write(
        steam.join("depotcache").join(manifest_file_name(&empty)),
        b"",
    )
    .expect("write empty manifest");
    let absent = pin(222, "2002");

    let report = verify_available(
        &steam.display().to_string(),
        987_654_321,
        &[present, empty.clone(), absent.clone()],
    );
    assert_eq!(report.verified, 1, "empty files are not local hits");
    assert_eq!(report.missing, vec![empty, absent]);

    let _ = std::fs::remove_dir_all(&steam);
}

#[test]
fn restore_blocking_counts_already_local_and_missing() {
    let steam = temp_steam_dir("restore");
    let present = pin(444, "4004");
    std::fs::write(
        steam.join("depotcache").join(manifest_file_name(&present)),
        b"payload",
    )
    .expect("write manifest");
    let absent = pin(555, "5005");

    let outcome = restore_local_blocking(
        &steam.display().to_string(),
        987_654_321,
        &[present, absent.clone()],
    )
    .expect("restore");
    assert_eq!(outcome.already_local, 1);
    assert_eq!(outcome.restored, 0);
    assert_eq!(outcome.missing, vec![absent]);

    let _ = std::fs::remove_dir_all(&steam);
}

#[test]
fn restore_blocking_is_idempotent_for_valid_files() {
    let steam = temp_steam_dir("idempotent");
    let present = pin(666, "6006");
    std::fs::write(
        steam.join("depotcache").join(manifest_file_name(&present)),
        b"payload",
    )
    .expect("write manifest");

    for _ in 0..2 {
        let outcome = restore_local_blocking(
            &steam.display().to_string(),
            987_654_321,
            &[present.clone()],
        )
        .expect("restore");
        assert_eq!(outcome.already_local, 1);
        assert_eq!(outcome.restored, 0);
        assert!(outcome.missing.is_empty());
    }

    let _ = std::fs::remove_dir_all(&steam);
}

// ============================================================================
// F2 — a failing pin must not stall the batch forever
// ============================================================================

/// The provider batch used to abort at the first failure, so one poisoned pin
/// made every pin of that game ungeneratable and the caller retried the same
/// batch on every poll. Failures are now per pin and, after a few consecutive
/// ones, that pin is quarantined instead of burning quota in a loop.
#[test]
fn quarantine_engages_only_after_consecutive_failures() {
    use crate::manifest::resolver::{
        clear_generation_failure, quarantine_streak, record_generation_failure,
        MAX_CONSECUTIVE_FAILURES,
    };

    // Unique ids: the failure memory is process-wide and shared with every
    // other test running in parallel.
    let poisoned = pin(900_001, "7000000000000000001");
    let healthy = pin(900_002, "7000000000000000002");
    clear_generation_failure(&poisoned);
    clear_generation_failure(&healthy);

    for attempt in 1..MAX_CONSECUTIVE_FAILURES {
        record_generation_failure(&poisoned);
        assert_eq!(
            quarantine_streak(&poisoned),
            None,
            "attempt {attempt} must still be retried"
        );
    }

    record_generation_failure(&poisoned);
    assert_eq!(
        quarantine_streak(&poisoned),
        Some(MAX_CONSECUTIVE_FAILURES),
        "the pin is skipped once the ladder is exhausted"
    );

    // The memory is per pin, never per batch: the healthy pin stays retryable.
    assert_eq!(quarantine_streak(&healthy), None);
    record_generation_failure(&healthy);
    assert_eq!(quarantine_streak(&healthy), None);

    // A success clears the streak: a provider that recovers is believed.
    clear_generation_failure(&poisoned);
    assert_eq!(quarantine_streak(&poisoned), None);
}

#[test]
fn quarantine_is_keyed_by_depot_and_manifest() {
    use crate::manifest::resolver::{
        clear_generation_failure, quarantine_streak, record_generation_failure,
        MAX_CONSECUTIVE_FAILURES,
    };

    let depot_a = pin(900_010, "7000000000000000010");
    let depot_b = pin(900_011, "7000000000000000010");
    let other_gid = pin(900_010, "7000000000000000011");
    for candidate in [&depot_a, &depot_b, &other_gid] {
        clear_generation_failure(candidate);
    }
    for _ in 0..MAX_CONSECUTIVE_FAILURES {
        record_generation_failure(&depot_a);
    }
    assert!(quarantine_streak(&depot_a).is_some());
    assert!(
        quarantine_streak(&depot_b).is_none(),
        "same GID on another depot is a different request"
    );
    assert!(
        quarantine_streak(&other_gid).is_none(),
        "same depot with another GID is a different request"
    );
}
