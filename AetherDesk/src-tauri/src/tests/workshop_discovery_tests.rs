//! Discovery contract for `appworkshop_*.acf`.
//!
//! Repair targets are only installed direct children with a positive manifest.
//! `WorkshopItemDetails` supplies pending subscriptions, never Hubcap targets.

use crate::commands::workshop::{discover_workshop_items, PendingWorkshopItem, WorkshopItem};
use std::fs;
use std::path::Path;

fn write_workshop_acf(root: &Path, app_id: u32, content: &str) {
    let dir = root.join("steamapps").join("workshop");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("appworkshop_{app_id}.acf")), content).unwrap();
}

fn write_content_dir(root: &Path, app_id: u32, workshop_id: u64, empty: bool) {
    let dir = root
        .join("steamapps")
        .join("workshop")
        .join("content")
        .join(app_id.to_string())
        .join(workshop_id.to_string());
    fs::create_dir_all(&dir).unwrap();
    if !empty {
        fs::write(dir.join("payload.bin"), b"payload").unwrap();
    }
}

fn installed_item(workshop_id: u64, manifest_gid: u64) -> String {
    format!("\t\t\"{workshop_id}\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"{manifest_gid}\"\n\t\t}}\n")
}

fn details_item(workshop_id: u64, manifest_gid: u64) -> String {
    format!(
        "\t\t\"{workshop_id}\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"{manifest_gid}\"\n\t\t\t\"latest_manifest\"\t\t\"{manifest_gid}\"\n\t\t}}\n"
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
    // The first real item carries the branch history that the old regex read
    // as Workshop IDs 0...19.
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

    let (items, pending) = discover_workshop_items(root.to_str().unwrap());

    assert_eq!(items.len(), 25);
    assert!(pending.is_empty());
    assert!(!items
        .iter()
        .any(|item| (0..=19).contains(&item.workshop_id)));
}

#[test]
fn separates_installed_from_pending() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let mut installed = String::new();
    for index in 1..=4u64 {
        installed.push_str(&installed_item(
            374_722_2630 + index,
            634_883_037_376_787_000 + index,
        ));
    }
    let details = details_item(3_806_068_832, 3_748_992_252_827_399_921);
    write_workshop_acf(root, 3328490, &installed_acf(&installed, &details, true));
    write_content_dir(root, 3328490, 3_806_068_832, true);

    let (items, pending) = discover_workshop_items(root.to_str().unwrap());

    assert_eq!(items.len(), 4);
    assert_eq!(
        pending,
        vec![PendingWorkshopItem {
            app_id: 3328490,
            workshop_id: 3_806_068_832,
            manifest_gid: 3_748_992_252_827_399_921,
            needs_download: true,
            content_empty: true,
        }]
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

    let (items, pending) = discover_workshop_items(root.to_str().unwrap());

    assert!(items.is_empty());
    assert!(pending.is_empty());
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

    let (items, pending) = discover_workshop_items(root.to_str().unwrap());

    assert!(pending.is_empty());
    assert_eq!(
        items,
        vec![WorkshopItem {
            app_id: 100,
            workshop_id: 555,
            manifest_gid: 111,
        }]
    );
}
