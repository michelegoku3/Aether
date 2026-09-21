use std::collections::HashMap;

use crate::core::hubcap_update_monitor::{
    acf_installed_gid, installed_gid_for_depot, latest_installed_gid, retry_delay, INITIAL_RETRY_DELAY,
    MAX_RETRY_DELAY,
};
use crate::manifest::pins::{DepotManifestPin, LuaManifestPins};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("aether_monitor_tests_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn retry_delay_is_bounded() {
    assert_eq!(retry_delay(0), INITIAL_RETRY_DELAY);
    assert_eq!(retry_delay(1), INITIAL_RETRY_DELAY * 2);
    assert_eq!(retry_delay(99), MAX_RETRY_DELAY);
}

#[test]
fn latest_installed_gid_picks_the_most_recent_non_empty_manifest() {
    let steam = temp_dir("gid");
    let depotcache = steam.join("depotcache");
    std::fs::create_dir_all(&depotcache).expect("depotcache");

    let old = depotcache.join("700_111.manifest");
    let new = depotcache.join("700_222.manifest");
    let empty = depotcache.join("700_333.manifest");
    std::fs::write(&old, b"old").expect("old write");
    std::fs::write(&new, b"new").expect("new write");
    std::fs::write(&empty, b"").expect("empty write");
    // The newest file must win even against files with a larger GID.
    let later = std::fs::OpenOptions::new()
        .append(true)
        .open(&new)
        .expect("open new");
    later.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600)).expect("mtime future");
    drop(later);

    assert_eq!(latest_installed_gid(&steam.display().to_string(), 700), Some("222".to_string()));
    // Empty manifests and other depots never qualify.
    assert_eq!(latest_installed_gid(&steam.display().to_string(), 701), None);

    let _ = std::fs::remove_dir_all(&steam);
}

#[test]
fn realign_updates_only_commented_pins_of_active_depots() {
    let steam = temp_dir("realign");
    let stplug = steam.join("config").join("stplug-in");
    std::fs::create_dir_all(&stplug).expect("stplug-in");
    let app_id = 123456u32;
    let lua = format!(
        concat!(
            "addappid({app}, 1, \"aa\")\n",
            "--setManifestid({app}, \"111\", 111)\n",
            "addappid(200, 1, \"bb\")\n",
            "--setManifestid(200, \"222\", 222)\n",
            "--addappid(300, 1, \"cc\")\n",
            "--setManifestid(300, \"333\", 333)\n",
        ),
        app = app_id
    );
    std::fs::write(stplug.join(format!("{app_id}.lua")), lua).expect("lua write");

    let editor = LuaManifestPins::new(steam.display().to_string(), app_id);
    let realigned = editor
        .realign_commented_pins(&[
            DepotManifestPin { depot_id: app_id, manifest_id: "999".to_string() },
            DepotManifestPin { depot_id: 200, manifest_id: "888".to_string() },
            DepotManifestPin { depot_id: 300, manifest_id: "777".to_string() },
        ])
        .expect("realign");

    // Only the two depots with an active addappid are realigned.
    assert_eq!(realigned, 2);
    let content = std::fs::read_to_string(stplug.join(format!("{app_id}.lua"))).expect("lua read");
    assert!(
        content.contains(&format!("--setManifestid({app_id}, \"999\")")),
        "active-depot pin must be updated and stay commented: {content}"
    );
    assert!(
        content.contains("--setManifestid(200, \"888\")"),
        "second active depot must be updated and stay commented: {content}"
    );
    assert!(
        content.contains("--setManifestid(300, \"333\""),
        "fully-disabled depot must be untouched: {content}"
    );
    // Row count preserved (safety check inside realign already asserts it).
    assert_eq!(LuaManifestPins::rows_from_content(&content).len(), 3);

    let _ = std::fs::remove_dir_all(&steam);
}

#[test]
fn realign_is_a_noop_when_the_gid_is_already_current() {
    let steam = temp_dir("noop");
    let stplug = steam.join("config").join("stplug-in");
    std::fs::create_dir_all(&stplug).expect("stplug-in");
    let app_id = 654321u32;
    std::fs::write(
        stplug.join(format!("{app_id}.lua")),
        format!("addappid({app_id}, 1, \"aa\")\n--setManifestid({app_id}, \"111\", 111)\n"),
    )
    .expect("lua write");

    let editor = LuaManifestPins::new(steam.display().to_string(), app_id);
    let realigned = editor
        .realign_commented_pins(&[DepotManifestPin { depot_id: app_id, manifest_id: "111".to_string() }])
        .expect("realign");
    assert_eq!(realigned, 0, "same GID must not rewrite the Lua");

    let _ = std::fs::remove_dir_all(&steam);
}

#[test]
fn changed_against_checkpoint_detects_new_and_changed_only() {
    let mut checkpoint = HashMap::new();
    checkpoint.insert(10u32, "old".to_string());
    checkpoint.insert(20u32, "same".to_string());
    let current = HashMap::from([
        (10u32, "new".to_string()),
        (20u32, "same".to_string()),
        (30u32, "fresh".to_string()),
    ]);
    let mut changed: Vec<u32> = current
        .iter()
        .filter(|(app_id, fp)| checkpoint.get(app_id) != Some(fp))
        .map(|(app_id, _)| *app_id)
        .collect();
    changed.sort_unstable();
    assert_eq!(changed, vec![10, 30]);
}

// ============================================================================
// F2 — bounded retry ladder
// ============================================================================

#[test]
fn retry_ladder_drops_a_task_after_the_cap() {
    use crate::core::hubcap_update_monitor::{reschedule, PendingTask, MAX_TASK_ATTEMPTS};

    let mut lane: std::collections::BTreeMap<u32, PendingTask> = std::collections::BTreeMap::new();
    let app_id = 700001u32;

    // Attempts below the cap keep the task queued...
    let mut task = PendingTask::due_now();
    for attempt in 1..MAX_TASK_ATTEMPTS {
        assert!(
            reschedule(&mut lane, app_id, task.clone()),
            "attempt {attempt} must stay queued"
        );
        assert!(lane.contains_key(&app_id));
        task = PendingTask::due_now();
    }

    // ...and the task is dropped once the ladder is exhausted, so one
    // permanently failing game can no longer occupy the lane forever.
    let last = PendingTask {
        attempts: MAX_TASK_ATTEMPTS - 1,
        next_attempt: std::time::Instant::now(),
    };
    assert!(!reschedule(&mut lane, app_id, last));
    assert!(lane.is_empty(), "a given-up task must not be rescheduled");
}

// ============================================================================
// F8 — contents-diff cadence
// ============================================================================

#[test]
fn contents_cadence_prefers_changed_games_and_skips_idle_ones() {
    use crate::core::hubcap_update_monitor::pin_refresh_candidates;

    let steam = temp_dir("cadence");
    let stplug = steam.join("config").join("stplug-in");
    std::fs::create_dir_all(&stplug).expect("stplug-in");
    let managed = "-- Game\naddappid(800001, 1, \"aa\")\n--setManifestid(800001, \"111\")\n";
    let locked = "addappid(800002, 1, \"aa\")\nsetManifestid(800002, \"222\")\n";
    std::fs::write(stplug.join("800001.lua"), managed).expect("write managed lua");
    std::fs::write(stplug.join("800002.lua"), locked).expect("write locked lua");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let current_luas = HashMap::from([
        (800001u32, "lua-a".to_string()),
        (800002u32, "lua-b".to_string()),
        // Not installed (no ACF): a moved GID could not be applied in place.
        (800003u32, "lua-c".to_string()),
    ]);
    let current_acf = HashMap::from([
        (800001u32, "acf-a-new".to_string()),
        (800002u32, "acf-b".to_string()),
    ]);

    // Both games were checked five minutes ago; only 800001 is installed AND
    // updates-enabled. Its Steam side changed since that check, so it is due on
    // the fast 15-minute cadence; 800002 is version-locked and must never be
    // diffed; 800003 has no ACF at all.
    let contents_checked = HashMap::from([(800001u32, now - 5 * 60), (800002u32, now - 5 * 60)]);
    let contents_fingerprint =
        HashMap::from([(800001u32, "acf-a-old".to_string()), (800002u32, "acf-b".to_string())]);

    let due = pin_refresh_candidates(
        &contents_checked,
        &contents_fingerprint,
        &current_acf,
        &current_luas,
        &steam.display().to_string(),
    );
    assert!(
        due.is_empty(),
        "five minutes after the last check nothing is due yet, got {due:?}"
    );

    // Quarter of an hour later the changed, updates-enabled game is due again.
    let later = HashMap::from([(800001u32, now - 16 * 60), (800002u32, now - 16 * 60)]);
    let due = pin_refresh_candidates(
        &later,
        &contents_fingerprint,
        &current_acf,
        &current_luas,
        &steam.display().to_string(),
    );
    assert_eq!(due, vec![800001], "only the changed, managed game is due");

    // The untouched game waits for the idle interval (1 h), not 15 minutes.
    let idle_checked = HashMap::from([(800001u32, now - 16 * 60)]);
    let idle_fingerprint = HashMap::from([(800001u32, "acf-a-new".to_string())]);
    let due = pin_refresh_candidates(
        &idle_checked,
        &idle_fingerprint,
        &current_acf,
        &current_luas,
        &steam.display().to_string(),
    );
    assert!(
        due.is_empty(),
        "an untouched game must not be re-diffed every 15 minutes, got {due:?}"
    );

    let old_checked = HashMap::from([(800001u32, now - 61 * 60)]);
    let due = pin_refresh_candidates(
        &old_checked,
        &idle_fingerprint,
        &current_acf,
        &current_luas,
        &steam.display().to_string(),
    );
    assert_eq!(due, vec![800001], "after an hour the idle game is re-checked");

    let _ = std::fs::remove_dir_all(&steam);
}

#[test]
fn installed_gid_prefers_steam_acf_over_depotcache_mtimes() {
    // Scenario behind "Disable updates": Steam has build X installed (ACF),
    // but depotcache holds a NEWER file for the same depot — a manifest that
    // was restored from backup, staged for another build, or downloaded for
    // an update Steam never applied. Locking must follow the ACF, otherwise
    // reactivating the pins would move the game to a version it does not have.
    let steam = temp_dir("acf_first");
    let app_id = 570940u32;
    let depot = 570941u32;
    let depotcache = steam.join("depotcache");
    std::fs::create_dir_all(&depotcache).expect("depotcache");
    std::fs::write(depotcache.join(format!("{depot}_111.manifest")), b"installed").expect("write");
    let stray = depotcache.join(format!("{depot}_999.manifest"));
    std::fs::write(&stray, b"stray").expect("write stray");
    let later = std::fs::OpenOptions::new().append(true).open(&stray).expect("open stray");
    later
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
        .expect("mtime future");
    drop(later);

    // Without an ACF (game not installed yet) the mtime heuristic is all there is.
    assert_eq!(acf_installed_gid(&steam.display().to_string(), app_id, depot), None);
    assert_eq!(
        installed_gid_for_depot(&steam.display().to_string(), app_id, depot).as_deref(),
        Some("999")
    );

    let steamapps = steam.join("steamapps");
    std::fs::create_dir_all(&steamapps).expect("steamapps");
    std::fs::write(
        steamapps.join(format!("appmanifest_{app_id}.acf")),
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"installdir\"\t\t\"Game\"\n\t\"InstalledDepots\"\n\t{{\n\t\t\"{depot}\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"111\"\n\t\t\t\"size\"\t\t\"10\"\n\t\t}}\n\t}}\n}}\n"
        ),
    )
    .expect("acf write");

    assert_eq!(acf_installed_gid(&steam.display().to_string(), app_id, depot).as_deref(), Some("111"));
    assert_eq!(
        installed_gid_for_depot(&steam.display().to_string(), app_id, depot).as_deref(),
        Some("111"),
        "Steam's InstalledDepots must win over a newer stray file in depotcache"
    );
    // A depot the ACF does not list still falls back to depotcache evidence.
    assert_eq!(installed_gid_for_depot(&steam.display().to_string(), app_id, 42), None);

    let _ = std::fs::remove_dir_all(&steam);
}
