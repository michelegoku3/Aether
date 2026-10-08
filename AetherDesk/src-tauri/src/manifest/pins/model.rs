use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaManifestRow {
    /// Zero-based line index of the `setManifestid` call in the Lua file.
    pub row_id: usize,
    pub app_id: u32,
    pub manifest_id: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<LuaManifestRowIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManifestGidProblem {
    NotDecimalUint64,
    MalformedCall,
}

impl ManifestGidProblem {
    pub fn reason(self) -> &'static str {
        match self {
            Self::NotDecimalUint64 => "the manifest GID must be a decimal uint64",
            Self::MalformedCall => {
                "the call must be setManifestid(<depot id>, \"<decimal gid>\")"
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaManifestRowIssue {
    /// One-based line number, matching Steam's Lua diagnostics.
    pub line: usize,
    pub depot_id: Option<u32>,
    pub raw_value: String,
    pub problem: ManifestGidProblem,
    /// Whether the malformed call is executable rather than commented out.
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaManifestEdit {
    pub row_id: usize,
    pub manifest_id: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DepotManifestPin {
    pub depot_id: u32,
    pub manifest_id: String,
}

pub fn pins_from_rows(rows: impl IntoIterator<Item = LuaManifestRow>) -> Vec<DepotManifestPin> {
    rows.into_iter()
        .map(|row| DepotManifestPin {
            depot_id: row.app_id,
            manifest_id: row.manifest_id,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyBuildResult {
    pub applied_pins: usize,
}

/// Parsed relationship between one manifest pin and its nearest `addappid`.
#[derive(Debug, Clone)]
pub(super) struct ParsedPin {
    pub row: LuaManifestRow,
    pub addappid_line: Option<usize>,
    pub addappid_enabled: bool,
    pub setmanifest_line: usize,
}
