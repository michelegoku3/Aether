use std::collections::HashMap;

use crate::manifest::pins::DepotManifestPin;
use crate::versioning::lua_validation::mismatching_changed_depots;

fn pin(depot_id: u32, manifest_id: &str) -> DepotManifestPin {
    DepotManifestPin {
        depot_id,
        manifest_id: manifest_id.to_string(),
    }
}

#[test]
fn accepts_every_manifest_changed_by_the_claimed_build() {
    let expected = vec![pin(10, "100"), pin(20, "200")];
    let actual = HashMap::from([(10, "100".to_string()), (20, "200".to_string())]);

    assert!(mismatching_changed_depots(&expected, &actual).is_empty());
}

#[test]
fn reports_different_and_missing_changed_depots_without_judging_extras() {
    let expected = vec![pin(10, "100"), pin(20, "200"), pin(30, "300")];
    let actual = HashMap::from([
        (10, "wrong".to_string()),
        (20, "200".to_string()),
        (999, "extra-is-allowed".to_string()),
    ]);

    assert_eq!(mismatching_changed_depots(&expected, &actual), vec![10, 30]);
}
