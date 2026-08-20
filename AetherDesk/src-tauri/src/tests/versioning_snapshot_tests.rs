//! Regression tests for reconstructing the provable portion of a build
//! snapshot from patch diffs. Unresolved depots are reported separately so the
//! writer can preserve their existing Lua state instead of disabling them.

use crate::manifest::pins::DepotManifestPin;
use crate::versioning::model::BuildInfo;
use crate::versioning::snapshot::{older_build_ids, SnapshotAssembler};

fn pin(depot_id: u32, manifest_id: &str) -> DepotManifestPin {
    DepotManifestPin {
        depot_id,
        manifest_id: manifest_id.to_string(),
    }
}

#[test]
fn reconstructs_unchanged_depots_from_nearest_older_builds() {
    let mut snapshot = SnapshotAssembler::new(&[101, 102, 103]).expect("non-empty depot set");

    snapshot.push_diff(&[pin(101, "target-101")]);
    snapshot.push_diff(&[pin(102, "recent-102")]);
    snapshot.push_diff(&[pin(101, "old-101"), pin(103, "recent-103")]);

    assert!(snapshot.is_complete());
    assert_eq!(
        snapshot.into_pins(),
        vec![
            pin(101, "target-101"),
            pin(102, "recent-102"),
            pin(103, "recent-103"),
        ]
    );
}

#[test]
fn ignores_depots_not_present_in_the_game_lua() {
    let mut snapshot = SnapshotAssembler::new(&[101]).expect("non-empty depot set");
    snapshot.push_diff(&[pin(101, "wanted"), pin(999, "unlisted")]);

    assert_eq!(snapshot.into_pins(), vec![pin(101, "wanted")]);
}

#[test]
fn reports_missing_depots_in_stable_order() {
    let snapshot = SnapshotAssembler::new(&[103, 101, 102]).expect("non-empty depot set");
    assert_eq!(snapshot.missing_depots(), vec![101, 102, 103]);
}

#[test]
fn selects_unique_older_builds_nearest_first() {
    let builds = vec![
        build(120),
        build(90),
        build(110),
        build(90),
        build(100),
    ];
    assert_eq!(older_build_ids(&builds, 111), vec![110, 100, 90]);
}

fn build(build_id: u64) -> BuildInfo {
    BuildInfo {
        build_id,
        date: String::new(),
        title: String::new(),
    }
}
