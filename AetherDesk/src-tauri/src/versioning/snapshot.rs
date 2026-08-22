use std::collections::{HashMap, HashSet};

use crate::manifest::pins::DepotManifestPin;
use crate::versioning::model::BuildInfo;

/// Deterministically reconstructs a depot snapshot from build patch diffs.
///
/// Diffs must be pushed newest-to-oldest. The first manifest observed for a
/// depot is therefore its value at the target build and is never overwritten
/// by an older manifest.
pub(crate) struct SnapshotAssembler {
    desired: HashSet<u32>,
    resolved: HashMap<u32, String>,
}

impl SnapshotAssembler {
    pub(crate) fn new(depot_ids: &[u32]) -> Option<Self> {
        let desired: HashSet<u32> = depot_ids.iter().copied().collect();
        if desired.is_empty() {
            return None;
        }
        Some(Self {
            desired,
            resolved: HashMap::new(),
        })
    }

    pub(crate) fn push_diff(&mut self, pins: &[DepotManifestPin]) {
        for pin in pins {
            if self.desired.contains(&pin.depot_id) {
                self.resolved
                    .entry(pin.depot_id)
                    .or_insert_with(|| pin.manifest_id.clone());
            }
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.resolved.len() == self.desired.len()
    }

    pub(crate) fn missing_depots(&self) -> Vec<u32> {
        let mut missing: Vec<u32> = self
            .desired
            .iter()
            .filter(|depot_id| !self.resolved.contains_key(depot_id))
            .copied()
            .collect();
        missing.sort_unstable();
        missing
    }

    pub(crate) fn into_pins(self) -> Vec<DepotManifestPin> {
        let mut pins: Vec<DepotManifestPin> = self
            .resolved
            .into_iter()
            .map(|(depot_id, manifest_id)| DepotManifestPin {
                depot_id,
                manifest_id,
            })
            .collect();
        pins.sort_by_key(|pin| pin.depot_id);
        pins
    }
}

/// Returns unique builds older than the target, nearest first. Steam BuildIDs
/// are globally monotonic, so this also works when a manually entered target
/// is valid but absent from the RSS response.
pub(crate) fn older_build_ids(builds: &[BuildInfo], target_build_id: u64) -> Vec<u64> {
    let mut ids: Vec<u64> = Vec::new();

    if let Some(target_idx) = builds.iter().position(|b| b.build_id == target_build_id) {
        for b in &builds[target_idx + 1..] {
            if b.build_id != target_build_id && !ids.contains(&b.build_id) {
                ids.push(b.build_id);
            }
        }
    }

    let mut remaining: Vec<u64> = builds
        .iter()
        .map(|b| b.build_id)
        .filter(|&id| id < target_build_id && !ids.contains(&id))
        .collect();
    remaining.sort_unstable_by(|a, b| b.cmp(a));
    ids.extend(remaining);
    ids.dedup();
    ids
}
