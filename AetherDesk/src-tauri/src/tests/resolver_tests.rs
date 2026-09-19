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
