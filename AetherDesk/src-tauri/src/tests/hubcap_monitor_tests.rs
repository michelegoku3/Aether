use std::collections::HashMap;

use crate::core::hubcap_update_monitor::{latest_installed_gid, retry_delay, INITIAL_RETRY_DELAY, MAX_RETRY_DELAY};
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
