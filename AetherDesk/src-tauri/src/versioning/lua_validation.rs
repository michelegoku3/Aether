use std::collections::HashMap;
use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::manifest::pins::{DepotManifestPin, LuaManifestPins};
use crate::versioning::sources::depotbox::DepotboxSource;
use crate::versioning::sources::BuildDetailsSource;

static BUILD_CLAIM_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)version-locked\s+to\s+build\s+(\d+)").expect("static build claim regex")
});

/// Validates only what can be proven from a build diff: every depot explicitly
/// changed by the claimed BuildID must carry that exact manifest in the Lua.
/// Unchanged depots cannot be validated from Depotbox and are intentionally not
/// judged. Provider mismatch never blocks installation; callers surface the
/// returned warning and preserve the generated Lua as the closest available
/// version.
pub async fn validate_claimed_build(
    lua_path: &Path,
    token: String,
) -> Result<Option<String>, String> {
    let content = std::fs::read_to_string(lua_path)
        .map_err(|error| format!("Could not read {} for build validation: {error}", lua_path.display()))?;
    let Some(captures) = BUILD_CLAIM_RE.captures(&content) else {
        return Ok(None);
    };
    let build_id = captures
        .get(1)
        .and_then(|value| value.as_str().parse::<u64>().ok())
        .ok_or_else(|| "The Lua build claim is not a valid BuildID".to_string())?;

    let expected = DepotboxSource::new(token).pins_for_build(build_id).await?;
    let actual: HashMap<u32, String> = LuaManifestPins::rows_from_content(&content)
        .into_iter()
        .map(|row| (row.app_id, row.manifest_id))
        .collect();

    let mismatches = mismatching_changed_depots(&expected, &actual);
    if mismatches.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        "Warning: this Lua claims Build {}, but {}/{} depot(s) changed by that build have different or missing manifests ({}). The provider appears to have returned a newer closest-available version; it was installed anyway.",
        build_id,
        mismatches.len(),
        expected.len(),
        mismatches
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

pub(crate) fn mismatching_changed_depots(
    expected: &[DepotManifestPin],
    actual: &HashMap<u32, String>,
) -> Vec<u32> {
    let mut mismatches: Vec<u32> = expected
        .iter()
        .filter(|pin| actual.get(&pin.depot_id) != Some(&pin.manifest_id))
        .map(|pin| pin.depot_id)
        .collect();
    mismatches.sort_unstable();
    mismatches.dedup();
    mismatches
}
