//! Discovery contract for `appworkshop_*.acf`.
//!
//! Installed items and `WorkshopItemDetails` subscriptions become repair
//! targets. Installed items prefer a newer `latest_manifest`; subscriptions
//! require a positive `manifest` and never use `latest_manifest` as a fallback.

use crate::commands::workshop::{discover_workshop_items, WorkshopTarget};
use std::fs;
use std::path::Path;

fn write_workshop_acf(root: &Path, app_id: u32, content: &str) {
    let dir = root.join("steamapps").join("workshop");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("appworkshop_{app_id}.acf")), content).unwrap();
}

fn installed_item(workshop_id: u64, manifest_gid: u64) -> String {
    format!("\t\t\"{workshop_id}\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"{manifest_gid}\"\n\t\t}}\n")
}

fn installed_item_at(workshop_id: u64, manifest_gid: u64, time_updated: u64) -> String {
    format!(
        "\t\t\"{workshop_id}\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"{manifest_gid}\"\n\t\t\t\"timeupdated\"\t\t\"{time_updated}\"\n\t\t}}\n"
    )
}

fn details_item(workshop_id: u64, manifest_gid: u64) -> String {
    format!(
        "\t\t\"{workshop_id}\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"{manifest_gid}\"\n\t\t\t\"latest_manifest\"\t\t\"{manifest_gid}\"\n\t\t}}\n"
    )
}

fn details_item_with_latest(
    workshop_id: u64,
    manifest_gid: u64,
    latest_manifest: u64,
    time_updated: u64,
    latest_time_updated: u64,
) -> String {
    format!(
        "\t\t\"{workshop_id}\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"{manifest_gid}\"\n\t\t\t\"timeupdated\"\t\t\"{time_updated}\"\n\t\t\t\"latest_manifest\"\t\t\"{latest_manifest}\"\n\t\t\t\"latest_timeupdated\"\t\t\"{latest_time_updated}\"\n\t\t}}\n"
    )
}

fn installed_acf(items: &str, details: &str, needs_download: bool) -> String {
    format!(
        "\"AppWorkshop\"\n{{\n\t\"appid\"\t\t\"1\"\n\t\"NeedsDownload\"\t\t\"{}\"\n\t\"WorkshopItemsInstalled\"\n\t{{\n{}\t}}\n\t\"WorkshopItemDetails\"\n\t{{\n{}\t}}\n}}\n",
        if needs_download { "1" } else { "0" },
        items,
        details
    )
}

#[test]
fn ignores_branch_history_indices() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let mut installed = String::new();
    for index in 1..=25u64 {
        installed.push_str(&installed_item(900_000 + index, 800_000 + index));
    }
    // The first real item carries a branch history one level below the item,
    // so a depth-aware parser must not treat its indices as Workshop IDs.
    let history = {
        let mut block = String::from("\t\t\t\"manifest_history\"\n\t\t\t{\n");
        for index in 0..=19u64 {
            block.push_str(&format!(
                "\t\t\t\t\"{index}\"\n\t\t\t\t{{\n\t\t\t\t\t\"manifest\"\t\t\"{}\"\n\t\t\t\t\t\"game_branch_min\"\t\t\"public\"\n\t\t\t\t}}\n",
                700_000 + index
            ));
        }
        block.push_str("\t\t\t}\n");
        block
    };
    installed = installed.replacen(
        "\t\t\t\"manifest\"\t\t\"800001\"\n",
        &format!("\t\t\t\"manifest\"\t\t\"800001\"\n{history}"),
        1,
    );
    write_workshop_acf(root, 2868840, &installed_acf(&installed, "", false));

    let targets = discover_workshop_items(root.to_str().unwrap());

    assert_eq!(targets.len(), 25);
    assert!(targets.iter().all(|target| !target.subscription));
    assert!(!targets
        .iter()
        .any(|target| (0..=19).contains(&target.item.workshop_id)));
}

#[test]
fn separates_installed_from_subscription_targets() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let mut installed = String::new();
    for index in 1..=4u64 {
        installed.push_str(&installed_item(
            3_747_222_630 + index,
            634_883_037_376_787_000 + index,
        ));
    }
    let details = details_item(3_806_068_832, 3_748_992_252_827_399_921);
    write_workshop_acf(root, 3328490, &installed_acf(&installed, &details, true));

    let targets = discover_workshop_items(root.to_str().unwrap());
    let installed_targets: Vec<&WorkshopTarget> = targets
        .iter()
        .filter(|target| !target.subscription)
        .collect();
    let subscription_targets: Vec<&WorkshopTarget> = targets
        .iter()
        .filter(|target| target.subscription)
        .collect();

    assert_eq!(installed_targets.len(), 4);
    assert_eq!(subscription_targets.len(), 1);
    assert_eq!(
        *subscription_targets[0],
        WorkshopTarget {
            item: crate::commands::workshop::WorkshopItem {
                app_id: 3328490,
                workshop_id: 3_806_068_832,
                manifest_gid: 3_748_992_252_827_399_921,
            },
            installed_manifest_gid: None,
            subscription: true,
            needs_download: true,
        }
    );
}

#[test]
fn skips_legacy_minus_one_manifests() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let mut installed = String::new();
    for index in 1..=21u64 {
        installed.push_str(&format!(
            "\t\t\"{}\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"-1\"\n\t\t\t\"ugchandle\"\t\t\"legacy\"\n\t\t}}\n",
            500_000 + index
        ));
    }
    write_workshop_acf(root, 241100, &installed_acf(&installed, "", false));

    let targets = discover_workshop_items(root.to_str().unwrap());

    assert!(targets.is_empty());
}

#[test]
fn deduplicates_by_item_id() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let root = first.path();
    let other = second.path();

    write_workshop_acf(
        root,
        100,
        &installed_acf(&installed_item(555, 111), "", false),
    );
    write_workshop_acf(
        other,
        200,
        &installed_acf(&installed_item(555, 222), "", false),
    );
    fs::create_dir_all(root.join("steamapps")).unwrap();
    fs::write(
        root.join("steamapps").join("libraryfolders.vdf"),
        format!(
            "\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
            other.to_string_lossy().replace('\\', "\\\\")
        ),
    )
    .unwrap();

    let targets = discover_workshop_items(root.to_str().unwrap());

    assert_eq!(
        targets,
        vec![WorkshopTarget {
            item: crate::commands::workshop::WorkshopItem {
                app_id: 100,
                workshop_id: 555,
                manifest_gid: 111,
            },
            installed_manifest_gid: Some(111),
            subscription: false,
            needs_download: false,
        }]
    );
}

#[test]
fn installed_items_are_not_subscriptions() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let installed = installed_item(3_747_222_633, 634_883_037_376_787_724);
    let details = details_item(3_747_222_633, 634_883_037_376_787_724);
    write_workshop_acf(root, 431960, &installed_acf(&installed, &details, false));

    let targets = discover_workshop_items(root.to_str().unwrap());

    assert_eq!(targets.len(), 1);
    assert!(!targets[0].subscription);
    assert_eq!(
        targets[0].installed_manifest_gid,
        Some(634_883_037_376_787_724)
    );
    assert!(!targets[0].is_update());
}

#[test]
fn uses_newer_latest_manifest_for_an_installed_update() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let installed = installed_item_at(555, 100, 10);
    let details = details_item_with_latest(555, 100, 200, 10, 20);
    write_workshop_acf(root, 1, &installed_acf(&installed, &details, true));

    let targets = discover_workshop_items(root.to_str().unwrap());

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].item.manifest_gid, 200);
    assert_eq!(targets[0].installed_manifest_gid, Some(100));
    assert!(targets[0].is_update());
}

#[test]
fn ignores_an_older_latest_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let installed = installed_item_at(555, 100, 20);
    let details = details_item_with_latest(555, 100, 200, 5, 10);
    write_workshop_acf(root, 1, &installed_acf(&installed, &details, false));

    let targets = discover_workshop_items(root.to_str().unwrap());

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].item.manifest_gid, 100);
    assert_eq!(targets[0].installed_manifest_gid, Some(100));
    assert!(!targets[0].is_update());
}

#[test]
fn needs_download_selects_latest_when_timestamps_are_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let installed = installed_item(555, 100);
    let details = "\t\t\"555\"\n\t\t{\n\t\t\t\"manifest\"\t\t\"100\"\n\t\t\t\"latest_manifest\"\t\t\"200\"\n\t\t}\n"
        .to_string();
    write_workshop_acf(root, 1, &installed_acf(&installed, &details, true));

    let targets = discover_workshop_items(root.to_str().unwrap());

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].item.manifest_gid, 200);
    assert!(targets[0].is_update());
}

#[test]
fn subscriptions_without_a_manifest_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let details = "\t\t\"555\"\n\t\t{\n\t\t\t\"latest_manifest\"\t\t\"200\"\n\t\t\t\"latest_timeupdated\"\t\t\"20\"\n\t\t}\n";
    write_workshop_acf(root, 1, &installed_acf("", details, true));

    assert!(discover_workshop_items(root.to_str().unwrap()).is_empty());
}
