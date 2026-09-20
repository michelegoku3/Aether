use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const HIDDEN_SYSTEM_DEPOTS: &[u32] = &[
    228981, 228982, 228983, 228984, 228985, 228986, 228987, 228988, 228989, 228990, 229000, 229001,
    229002, 229003, 229004, 229005, 229006, 229007, 229010, 229011, 229012, 229020, 229030, 229031,
    229032, 229033,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaManifestRow {
    /// Current line index of the setManifestid call in the Lua file.
    pub row_id: usize,
    pub app_id: u32,
    pub manifest_id: String,
    pub enabled: bool,
    /// Set only for rows that the strict parser rejects: the row exists in the
    /// file (and is shown to the user so it can be repaired) but AetherDLL
    /// cannot load the file at all while it stays malformed. `None` on every
    /// well-formed row, so serialized payloads are unchanged for healthy luas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<LuaManifestRowIssue>,
}

/// Why a `setManifestid` line cannot be used.
///
/// The Lua binding in AetherDLL (`setmanifestid: gid must be a decimal uint64`
/// in `scripting/LuaBindings.cpp`) raises on the first bad call and the script
/// engine rolls the whole file back, so a malformed line is a **file-wide**
/// problem, not a row-level warning: every depot of that game loses its
/// override. The type exists so the UI can say precisely what is wrong instead
/// of only that something is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManifestGidProblem {
    /// The call is well shaped and quoted, but the value is not a decimal
    /// uint64 (hex, leading/trailing spaces, exponent notation, overflow,
    /// empty string, …).
    NotDecimalUint64,
    /// The call shape itself is wrong (unquoted value, missing quotes or
    /// parenthesis, non-numeric depot id), so the argument cannot even be
    /// located unambiguously.
    MalformedCall,
}

impl ManifestGidProblem {
    /// Human sentence used in both the strict validation error and the UI hint.
    pub fn reason(self) -> &'static str {
        match self {
            ManifestGidProblem::NotDecimalUint64 => {
                "the manifest GID must be a decimal uint64"
            }
            ManifestGidProblem::MalformedCall => {
                "the call must be setManifestid(<depot id>, \"<decimal gid>\")"
            }
        }
    }
}

/// One malformed `setManifestid` line, ready to be shown (or repaired) by the
/// editor. 1-based `line` matches what Steam's own Lua error reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaManifestRowIssue {
    /// 1-based line number in the Lua file.
    pub line: usize,
    /// Depot id read from the first argument, when it is readable.
    pub depot_id: Option<u32>,
    /// Offending argument text exactly as it appears in the file.
    pub raw_value: String,
    pub problem: ManifestGidProblem,
    /// True when the call is NOT commented out, i.e. the script engine will
    /// actually execute it and raise. This is the severity that matters:
    ///  * active issue  -> AetherDLL rejects the whole file: the game loses
    ///    every depot override (red card, must be repaired);
    ///  * inactive issue -> the file loads fine today, but the call is a trap
    ///    for whenever updates are enabled (amber, repair when convenient).
    /// The field is what lets one diagnostic describe both states honestly
    /// instead of treating a commented line like a fatal one.
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaManifestEdit {
    pub row_id: usize,
    pub manifest_id: Option<String>,
    pub enabled: bool,
}

/// One (depot, manifest) pair of a game version snapshot. Manifest GIDs are
/// kept as strings: they exceed 2^53, so numeric transport would lose
/// precision (same rule SFF follows).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DepotManifestPin {
    pub depot_id: u32,
    pub manifest_id: String,
}

/// Converts manifest rows into the exact (depot, manifest) pins used by the
/// resolver and the backup pipeline. Row filtering (enabled rows, addappid
/// active rows, …) stays with the caller: different pipelines sync different
/// subsets of a Lua.
pub fn pins_from_rows(rows: impl IntoIterator<Item = LuaManifestRow>) -> Vec<DepotManifestPin> {
    rows.into_iter()
        .map(|row| DepotManifestPin {
            depot_id: row.app_id,
            manifest_id: row.manifest_id,
        })
        .collect()
}

/// Outcome of `LuaManifestPins::apply_build_pins`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyBuildResult {
    /// Number of pins written into the Lua.
    pub applied_pins: usize,
}

#[derive(Debug, Clone)]
struct ManifestPin {
    row: LuaManifestRow,
    addappid_line: Option<usize>,
    addappid_enabled: bool,
    setmanifest_line: usize,
}

/// Any `setManifestid(` call, commented or not — used to recognise the lines
/// the runtime will execute (and therefore validate) even when their arguments
/// are malformed.
static SETMANIFEST_ANY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^\s*(?:--\s*)?setmanifestid\s*\(").expect("valid regex"));

/// The canonical, well-formed call with a quoted GID. This is the only shape
/// that produces an editable row.
static SETMANIFEST_CALL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"^\s*(?P<comment>--\s*)?(?i:setmanifestid)\s*\(\s*(?P<appid>\d+)\s*,\s*["'](?P<manifest>[^"']+)["']\s*(?:,[^)]*)?\)"#,
    )
    .expect("valid regex")
});

/// Rewrite shape: like the canonical call, but capturing the pieces that must
/// be preserved (indentation, comment marker, function spelling, trailing text).
static SETMANIFEST_REWRITE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"^(?P<indent>\s*)(?P<comment>--\s*)?(?P<func>(?i:setmanifestid))\s*\(\s*(?P<appid>\d+)\s*,\s*["'][^"']+["']\s*(?:,[^)]*)?\)(?P<suffix>.*)$"#,
    )
    .expect("valid regex")
});

/// `addappid(<depot>, …)` with an optional comment marker.
static ADDAPPID_CALL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*(?P<comment>--\s*)?(?i:addappid)\s*\(\s*(?P<appid>\d+)\b")
        .expect("valid regex")
});

/// First argument (depot id) of a `setManifestid` call, when it is numeric.
static SETMANIFEST_DEPOT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:--\s*)?setmanifestid\s*\(\s*(?P<appid>\d+)").expect("valid regex")
});

/// Shape-only match for `setManifestid(<digits>, "<anything>")`.
///
/// It exists to separate the two failure modes: a call that *has* a value which
/// happens to be unusable (not a decimal uint64) versus a call whose shape is
/// broken. The first one can be repaired by editing the value in place, the
/// second one has to be rewritten — and the UI says which.
static SETMANIFEST_SHAPE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        // A depot id, a comma, then a VALUE — quoted (`"0x1234"`, `"abc"`, even
        // `""`) or an unquoted token (`0x1234`). Both are values the Lua
        // binding refuses with the same "must be a decimal uint64" message, so
        // both must be described as a value problem. What is NOT a value is a
        // missing second argument (`setManifestid(3558402)`) or a broken
        // parenthesis: those have nothing to repair in place.
        r#"^\s*(?:--\s*)?(?i:setmanifestid)\s*\(\s*\d+\s*,\s*(?:["'][^"']*["']|[^,)\s][^,)]*)\s*(?:,[^)]*)?\)"#,
    )
    .expect("valid regex")
});

/// Best-effort `(depot id, value)` of a `setManifestid` call, tolerant of every
/// shape the strict parser rejects — that is the whole point: it runs on the
/// lines that have to be *described*. Returns `None` when even the parenthesis
/// or the comma is missing.
fn loose_setmanifest_args(line: &str) -> Option<(Option<u32>, Option<String>)> {
    let trimmed = line.trim_start();
    let body = trimmed.strip_prefix("--").map(str::trim_start).unwrap_or(trimmed);
    let open = body.find('(')?;
    let close = body[open..].find(')').map(|offset| open + offset)?;
    let args = &body[open + 1..close];
    let mut parts = args.splitn(2, ',');
    let depot_raw = parts.next().unwrap_or_default().trim();
    let depot_id = depot_raw.parse::<u32>().ok();
    // The value is kept VERBATIM inside its quotes: a leading/trailing space is
    // one of the ways a GID is wrong, and it is invisible unless it is shown.
    let value = parts.next().map(|value| {
        let value = value.trim();
        if let Some(rest) = value.strip_prefix('"') {
            return rest.split('"').next().unwrap_or_default().to_string();
        }
        if let Some(rest) = value.strip_prefix('\'') {
            return rest.split('\'').next().unwrap_or_default().to_string();
        }
        // Unquoted token: ends at the next comma (a third argument may follow).
        value.split(',').next().unwrap_or_default().trim().to_string()
    });
    Some((depot_id, value))
}

/// The malformed lines that actually break the file today, i.e. those the
/// script engine will execute. Used by the library card: a commented bad call
/// is worth knowing about (the editor shows it in amber) but it is not a
/// reason to mark a working game as broken.
pub fn active_invalid_manifest_rows(content: &str) -> Vec<LuaManifestRowIssue> {
    invalid_manifest_rows(content)
        .into_iter()
        .filter(|issue| issue.active)
        .collect()
}

/// Every malformed `setManifestid` line in `content`, in file order.
///
/// The definition of "malformed" is the strict parser itself: a line is either
/// a valid row (`parse_setmanifest_line` succeeds) or an issue. Callers that
/// previously only saw "invalid file" now get the offending line, the depot id
/// and the raw value, which is what the editor needs to offer a repair.
pub fn invalid_manifest_rows(content: &str) -> Vec<LuaManifestRowIssue> {
    let mut issues = Vec::new();
    for (line_index, line) in content.lines().enumerate() {
        if !SETMANIFEST_ANY_RE.is_match(line) {
            continue;
        }
        if LuaManifestPins::parse_setmanifest_line(line).is_some() {
            continue;
        }
        let (depot_id, raw_value) = loose_setmanifest_args(line)
            .map(|(depot_id, value)| (depot_id, value))
            .unwrap_or((None, None));
        // A call that matches the strict *shape* but not the value rule has a
        // broken GID; anything else is a broken call. The difference decides
        // whether the editor can repair the value in place.
        let problem = if SETMANIFEST_SHAPE_RE.is_match(line) {
            ManifestGidProblem::NotDecimalUint64
        } else {
            ManifestGidProblem::MalformedCall
        };
        issues.push(LuaManifestRowIssue {
            line: line_index + 1,
            depot_id,
            raw_value: raw_value.unwrap_or_default(),
            problem,
            active: !line.trim_start().starts_with("--"),
        });
    }
    issues
}

pub struct LuaManifestPins {
    lua_path: PathBuf,
}

impl LuaManifestPins {
    pub fn new(steam_path: impl Into<PathBuf>, root_app_id: u32) -> Self {
        // Normalize at the boundary: callers strict-validate at command entry
        // for actionable errors, while the editor itself stays usable with any
        // raw-but-normalizable input (quoted copy-paste, trailing spaces).
        let normalized =
            crate::steam::resolve::normalize_steam_path(&steam_path.into().to_string_lossy());
        Self {
            lua_path: PathBuf::from(normalized)
                .join("config")
                .join("stplug-in")
                .join(format!("{}.lua", root_app_id)),
        }
    }

    /// Path of the `<appid>.lua` file this editor operates on.
    pub fn lua_path(&self) -> &std::path::Path {
        &self.lua_path
    }

    pub fn path_exists(&self) -> bool {
        self.lua_path.exists()
    }


    /// Applies the resolved portion of a build snapshot to the Lua: every
    /// listed depot gets its manifest pinned and is enabled. Depots absent from
    /// `pins` are deliberately left untouched because the finite history may
    /// simply not reach their last update; absence is never interpreted as
    /// removal. The row count is verified before saving.
    pub fn apply_build_pins(&self, pins: &[DepotManifestPin]) -> Result<ApplyBuildResult, String> {
        let content = self.read_lua()?;
        Self::validate_content(&content)?;
        let current = Self::pins_from_content(&content);
        let before_count = current.len();
        let mut lines: Vec<String> = content.lines().map(str::to_string).collect();

        let wanted: std::collections::HashMap<u32, &str> = pins
            .iter()
            .map(|pin| (pin.depot_id, pin.manifest_id.as_str()))
            .collect();

        let mut applied = 0usize;

        for pin in &current {
            match wanted.get(&pin.row.app_id) {
                Some(manifest_id) => {
                    let old_manifest = &pin.row.manifest_id;
                    if let Some(addappid_line) = pin.addappid_line {
                        Self::set_commented(&mut lines[addappid_line], false);
                    }
                    Self::set_commented(&mut lines[pin.setmanifest_line], false);
                    if *manifest_id != pin.row.manifest_id {
                        crate::desk_log_info!(
                            "manifest",
                            "Depot {}: manifest modified from '{}' to '{}'",
                            pin.row.app_id,
                            old_manifest,
                            manifest_id
                        );
                        lines[pin.setmanifest_line] = Self::rewrite_setmanifest_line(
                            &lines[pin.setmanifest_line],
                            manifest_id,
                        )?;
                    } else {
                        crate::desk_log_info!(
                            "manifest",
                            "Depot {}: manifest unchanged ('{}')",
                            pin.row.app_id,
                            manifest_id
                        );
                    }
                    applied += 1;
                }
                None => {
                    crate::desk_log_info!(
                        "manifest",
                        "Depot {}: absent from build snapshot, preserved as-is ('{}')",
                        pin.row.app_id,
                        pin.row.manifest_id
                    );
                }
            }
        }

        let next_content = Self::join_lua_lines(&lines);
        let after_count = Self::pins_from_content(&next_content).len();
        if after_count != before_count {
            return Err(format!(
                "Safety check failed: setManifestid count changed from {} to {}. File was not saved.",
                before_count, after_count
            ));
        }

        self.write_lua(&next_content)?;
        crate::desk_log_info!(
            "manifest",
            "Lua manifest {}: apply_build_pins completed -> {} pin(s) applied, {} depot(s) absent from the supplied snapshot and safely left unchanged",
            self.lua_path.display(),
            applied,
            current.len() - applied
        );
        Ok(ApplyBuildResult {
            applied_pins: applied,
        })
    }

    /// Updates the GID of commented `setManifestid` rows whose `addappid` is
    /// still active to the exact pins supplied, leaving every row's
    /// commented/enabled state exactly as it was. Used by the background
    /// synchronizer after Steam completed a build update: active pins are the
    /// user's explicit version lock and are never touched, and fully-disabled
    /// depots (both lines commented) stay untouched too. The row count is
    /// verified before saving, mirroring `apply_build_pins`.
    pub fn realign_commented_pins(&self, pins: &[DepotManifestPin]) -> Result<usize, String> {
        let content = self.read_lua()?;
        Self::validate_content(&content)?;
        let current = Self::pins_from_content(&content);
        let before_count = current.len();
        let wanted: std::collections::HashMap<u32, &str> = pins
            .iter()
            .map(|pin| (pin.depot_id, pin.manifest_id.as_str()))
            .collect();
        let mut lines: Vec<String> = content.lines().map(str::to_string).collect();

        let mut realigned = 0usize;
        for pin in &current {
            // Only informational (commented) pins of depots the Lua still
            // manages are candidates: the comment must survive the rewrite.
            if pin.row.enabled || !pin.addappid_enabled {
                continue;
            }
            let Some(manifest_id) = wanted.get(&pin.row.app_id) else {
                continue;
            };
            if *manifest_id == pin.row.manifest_id {
                continue;
            }
            lines[pin.setmanifest_line] = Self::rewrite_setmanifest_line(
                &lines[pin.setmanifest_line],
                manifest_id,
            )?;
            realigned += 1;
        }
        if realigned == 0 {
            return Ok(0);
        }

        let next_content = Self::join_lua_lines(&lines);
        if Self::pins_from_content(&next_content).len() != before_count {
            return Err(format!(
                "Safety check failed: setManifestid count changed from {} to {}. File was not saved.",
                before_count,
                Self::pins_from_content(&next_content).len()
            ));
        }
        self.write_lua(&next_content)?;
        Ok(realigned)
    }

    /// SFF-style extraction: do not execute or normalize the Lua; only scan text lines
    /// for setManifestid(depot, "gid", optional_size) calls.
    pub fn rows_from_content(content: &str) -> Vec<LuaManifestRow> {
        let mut rows: Vec<LuaManifestRow> = Self::pins_from_content(content)
            .into_iter()
            .map(|pin| pin.row)
            .filter(|row| !HIDDEN_SYSTEM_DEPOTS.contains(&row.app_id))
            .collect();
        rows.sort_by_key(|row| row.app_id);
        rows
    }

    /// Rows belonging to active `addappid` entries, including commented
    /// `setManifestid` pins. The latter are exactly the pins that Steam is
    /// allowed to update while Aether is in update mode, so manifest repair
    /// must not confuse "updates enabled" with "no manifest required".
    pub fn rows_for_manifest_sync(content: &str) -> Vec<LuaManifestRow> {
        let mut rows: Vec<LuaManifestRow> = Self::pins_from_content(content)
            .into_iter()
            .filter(|pin| pin.addappid_enabled)
            .map(|pin| pin.row)
            .filter(|row| !HIDDEN_SYSTEM_DEPOTS.contains(&row.app_id))
            .collect();
        rows.sort_by_key(|row| row.app_id);
        rows
    }

    /// Validates every setManifestid line before a provider or local importer
    /// writes the Lua to Steam. Commented pins are checked too: enabling
    /// updates later would execute them, so silently accepting a malformed
    /// commented GID only defers the failure to the DLL hot-reload path.
    ///
    /// This is the write gate: it is built on [`invalid_manifest_rows`], the
    /// single definition of "malformed line", so the message a user sees on a
    /// failed install and the diagnosis the editor shows can never disagree.
    pub fn validate_content(content: &str) -> Result<(), String> {
        match invalid_manifest_rows(content).into_iter().next() {
            Some(issue) => Err(format!(
                "Invalid setManifestid call at Lua line {}: {}",
                issue.line,
                issue.problem.reason()
            )),
            None => Ok(()),
        }
    }

    /// Install/import gate: refuses a file with ANY malformed pin, commented
    /// included. A commented malformed call cannot break today's load, but it
    /// is exactly what breaks the moment updates are enabled (the editor
    /// uncomments pins), so it must not enter the library silently.
    ///
    /// [`Self::validate_executable_content`] is the narrower rule used after a
    /// user repair, where commenting a broken call out IS the fix.
    pub fn validate_executable_content(content: &str) -> Result<(), String> {
        // Built on the active-only selector so "what breaks the file today" is
        // defined once, next to the rule, and shared with the UI ask.
        match active_invalid_manifest_rows(content).into_iter().next() {
            Some(issue) => Err(format!(
                "Invalid setManifestid call at Lua line {}: {}",
                issue.line,
                issue.problem.reason()
            )),
            None => Ok(()),
        }
    }

    /// First *executable* malformed pin: the reason AetherDLL would reject the
    /// whole file. `None` for a loadable Lua (including one whose only
    /// malformed pins are commented out).
    ///
    /// Pipelines that rewrite a Lua use this to stop BEFORE retrying: a file
    /// nobody can load is a user-fixable condition, not a transient failure,
    /// and reporting it once beats a bounded-but-pointless retry ladder.
    pub fn first_active_issue(content: &str) -> Option<LuaManifestRowIssue> {
        active_invalid_manifest_rows(content).into_iter().next()
    }

    /// Strict read: refuses a file with malformed pins. Used where the file is
    /// *verified* (installation checks) rather than displayed.
    pub fn rows_from_file(&self) -> Result<Vec<LuaManifestRow>, String> {
        let content = self.read_lua()?;
        Self::validate_content(&content)?;
        Ok(Self::rows_from_content(&content))
    }

    /// Tolerant read for the editor: every well-formed row **plus** one row per
    /// malformed line, marked with its [`LuaManifestRowIssue`]. The editor can
    /// therefore open (and repair) a file the strict gate would reject, instead
    /// of failing before the user can see what is wrong.
    pub fn editor_rows_from_file(&self) -> Result<Vec<LuaManifestRow>, String> {
        let content = self.read_lua()?;
        Ok(Self::editor_rows_from_content(&content))
    }

    /// Row list shown by the editor: valid rows ∪ malformed lines, ordered the
    /// way the table renders them (depot id, then file position).
    pub fn editor_rows_from_content(content: &str) -> Vec<LuaManifestRow> {
        let mut rows = Self::rows_from_content(content);
        rows.extend(
            invalid_manifest_rows(content)
                .into_iter()
                .map(Self::issue_row),
        );
        rows.sort_by_key(|row| (row.app_id, row.row_id));
        rows
    }

    /// A malformed line as a row: the raw text is carried in `manifest_id` so
    /// the editor can show the value that has to be replaced.
    fn issue_row(issue: LuaManifestRowIssue) -> LuaManifestRow {
        LuaManifestRow {
            row_id: issue.line.saturating_sub(1),
            app_id: issue.depot_id.unwrap_or(0),
            manifest_id: issue.raw_value.clone(),
            enabled: false,
            issue: Some(issue),
        }
    }

    /// Every game depot managed by the version editor. Steam's hidden system
    /// depots are intentionally excluded: they do not belong to the game's
    /// build history and therefore cannot be reconstructed from its patches.
    pub fn depot_ids_from_file(&self) -> Result<Vec<u32>, String> {
        let content = self.read_lua()?;
        let mut depot_ids: Vec<u32> = Self::rows_from_content(&content)
            .into_iter()
            .map(|row| row.app_id)
            .collect();
        depot_ids.sort_unstable();
        depot_ids.dedup();
        Ok(depot_ids)
    }

    pub fn updates_are_enabled(&self) -> Result<bool, String> {
        let content = self.read_lua()?;
        // Deliberately tolerant: a malformed line is reported by the editor and
        // the library card (both surface it as a fixable problem), while this
        // predicate must still answer for the *well-formed* rows of the file.
        // Making it fail here turned an unrelated malformed line into a hard
        // error for the update toggle, the manifest repair and the background
        // synchronizer (which then retried that game forever).
        let lines: Vec<&str> = content.lines().collect();

        // Updates are considered enabled only when at least one setManifestid pin
        // is commented while its related addappid is still enabled. If both addappid
        // and setManifestid are commented, that depot is disabled by the version
        // editor and must not affect the global Enable/Disable Update button.
        Ok(Self::pins_from_content(&content)
            .into_iter()
            .filter(|pin| pin.addappid_enabled)
            .any(|pin| {
                lines
                    .get(pin.setmanifest_line)
                    .map(|line| line.trim_start().starts_with("--"))
                    .unwrap_or(false)
            }))
    }

    pub fn set_updates_enabled(&self, enabled: bool) -> Result<usize, String> {
        let content = self.read_lua()?;
        Self::validate_content(&content)?;
        let pins = Self::pins_from_content(&content);
        let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
        let mut changed = 0usize;

        for pin in &pins {
            // Enable/Disable Update acts only on depots whose addappid is active.
            // Depots disabled through Change Version have their addappid commented;
            // their setManifestid must stay commented and untouched.
            if !pin.addappid_enabled {
                continue;
            }

            let before = lines[pin.setmanifest_line].clone();
            Self::set_commented(&mut lines[pin.setmanifest_line], enabled);
            if lines[pin.setmanifest_line] != before {
                changed += 1;
            }
        }

        let next_content = Self::join_lua_lines(&lines);
        let after_count = Self::pins_from_content(&next_content).len();
        if after_count != pins.len() {
            return Err(format!(
                "Safety check failed: setManifestid count changed from {} to {}. File was not saved.",
                pins.len(), after_count
            ));
        }

        if changed > 0 {
            self.write_lua(&next_content)?;
        }
        crate::desk_log_info!(
            "manifest",
            "Lua manifest {}: set_updates_enabled({}) completed -> {} pin(s) modified",
            self.lua_path.display(),
            enabled,
            changed
        );
        Ok(changed)
    }

    /// Builds the edited Lua in memory and validates it without writing it.
    /// Callers that need remote manifest generation can therefore complete all
    /// local/provider checks before Steam observes the new pins.
    pub fn preview_edits(
        &self,
        edits: &[LuaManifestEdit],
    ) -> Result<(String, Vec<LuaManifestRow>), String> {
        let content = self.read_lua()?;
        // The INPUT is read tolerantly (a malformed line is exactly what an
        // edit may be here to repair) while the OUTPUT is still gated by
        // `validate_content`: a repair writes a valid file, and an edit that
        // would leave a malformed line next to valid ones is refused.
        let pins = Self::pins_from_content(&content);
        let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
        let issues = invalid_manifest_rows(&content);
        let before_total = pins.len() + issues.len();

        for edit in edits {
            if let Some(pin) = pins.iter().find(|pin| pin.row.row_id == edit.row_id) {
                if let Some(addappid_line) = pin.addappid_line {
                    Self::set_commented(&mut lines[addappid_line], !edit.enabled);
                }
                Self::set_commented(&mut lines[pin.setmanifest_line], !edit.enabled);

                if let Some(next_manifest_id) = edit.manifest_id.as_deref().map(str::trim) {
                    if !next_manifest_id.is_empty() && next_manifest_id != pin.row.manifest_id {
                        Self::ensure_decimal_uint64(next_manifest_id)?;
                        crate::desk_log_info!(
                            "manifest",
                            "Depot {}: manifest modified from '{}' to '{}' (manual edit)",
                            pin.row.app_id,
                            pin.row.manifest_id,
                            next_manifest_id
                        );
                        lines[pin.setmanifest_line] = Self::rewrite_setmanifest_line(
                            &lines[pin.setmanifest_line],
                            next_manifest_id,
                        )?;
                    }
                }
                continue;
            }

            // Not a valid pin: it may be one of the malformed lines this very
            // edit is meant to fix. Repairing keeps the file usable instead of
            // forcing the user to edit Lua by hand.
            let Some(issue) = issues
                .iter()
                .find(|issue| issue.line.saturating_sub(1) == edit.row_id)
            else {
                return Err(format!("setManifestid row {} was not found", edit.row_id));
            };

            let Some(target) = lines.get_mut(edit.row_id) else {
                return Err(format!("setManifestid row {} was not found", edit.row_id));
            };

            if !edit.enabled {
                // Disabling is a legitimate resolution: commenting the call out
                // removes it from the executed script.
                Self::set_commented(target, true);
                crate::desk_log_info!(
                    "manifest",
                    "Lua line {}: malformed setManifestid commented out (depot {:?})",
                    issue.line,
                    issue.depot_id
                );
                continue;
            }

            let Some(next_manifest_id) = edit.manifest_id.as_deref().map(str::trim) else {
                continue;
            };
            if next_manifest_id.is_empty() {
                continue;
            }
            Self::ensure_decimal_uint64(next_manifest_id)?;
            *target = Self::rewrite_setmanifest_line(target, next_manifest_id)?;
            crate::desk_log_info!(
                "manifest",
                "Lua line {}: malformed setManifestid '{}' repaired to '{}'",
                issue.line,
                issue.raw_value,
                next_manifest_id
            );
        }

        let next_content = Self::join_lua_lines(&lines);
        // Count both families so a repair (malformed → well formed) does not
        // look like a lost row. For healthy files this is the same check as
        // before: `issues` is empty and only valid pins are counted.
        let after_total = Self::pins_from_content(&next_content).len()
            + invalid_manifest_rows(&next_content).len();
        if after_total != before_total {
            return Err(format!(
                "Safety check failed: setManifestid count changed from {} to {}. File was not saved.",
                before_total, after_total
            ));
        }
        // Output gate: no *executable* malformed call may remain. A commented
        // one is the accepted outcome of "turn this row off" and is still
        // reported to the UI as an inactive issue.
        Self::validate_executable_content(&next_content)?;
        let rows = Self::rows_from_content(&next_content);
        Ok((next_content, rows))
    }



    fn pins_from_content(content: &str) -> Vec<ManifestPin> {
        let lines: Vec<&str> = content.lines().collect();
        let mut pins = Vec::new();

        for (line_index, line) in lines.iter().enumerate() {
            let Some((app_id, manifest_id, setmanifest_commented)) =
                Self::parse_setmanifest_line(line)
            else {
                continue;
            };
            let addappid_line = Self::find_nearest_addappid(&lines, line_index, app_id);
            let addappid_enabled = addappid_line
                .and_then(|index| {
                    Self::parse_addappid_line(lines[index]).map(|(_, commented)| !commented)
                })
                .unwrap_or(!setmanifest_commented);

            pins.push(ManifestPin {
                row: LuaManifestRow {
                    row_id: line_index,
                    app_id,
                    manifest_id,
                    enabled: !setmanifest_commented && addappid_enabled,
                    issue: None,
                },
                addappid_line,
                addappid_enabled,
                setmanifest_line: line_index,
            });
        }

        pins
    }

    /// Rejects anything AetherDLL's `setmanifestid` binding would reject, so a
    /// value accepted here is a value the DLL can execute. Returning `Err`
    /// keeps the caller's message actionable ("what"), the issue list gives the
    /// editor the same rule with "where".
    pub(crate) fn ensure_decimal_uint64(value: &str) -> Result<(), String> {
        if value.is_empty()
            || value.parse::<u64>().is_err()
            || !value.chars().all(|character| character.is_ascii_digit())
        {
            return Err(format!(
                "Invalid manifest GID '{}': {}",
                value,
                ManifestGidProblem::NotDecimalUint64.reason()
            ));
        }
        Ok(())
    }

    fn parse_setmanifest_line(line: &str) -> Option<(u32, String, bool)> {
        // Supports:
        // setManifestid(3764201, "299...", 123)
        // setmanifestid(3764201, '299...')
        // --setManifestid(...)
        // -- setManifestid(...)
        let caps = SETMANIFEST_CALL_RE.captures(line)?;
        let app_id = caps.name("appid")?.as_str().parse().ok()?;
        let manifest_id = caps.name("manifest")?.as_str();
        // Steam's Lua binding accepts decimal uint64 GIDs only. Keep the
        // editor and backup pipeline aligned with that runtime contract and
        // reject hex/text/overflow values instead of exposing unusable rows.
        Self::ensure_decimal_uint64(manifest_id).ok()?;
        Some((
            app_id,
            manifest_id.to_string(),
            caps.name("comment").is_some(),
        ))
    }

    fn parse_addappid_line(line: &str) -> Option<(u32, bool)> {
        let caps = ADDAPPID_CALL_RE.captures(line)?;
        Some((
            caps.name("appid")?.as_str().parse().ok()?,
            caps.name("comment").is_some(),
        ))
    }

    fn find_nearest_addappid(
        lines: &[&str],
        setmanifest_line: usize,
        app_id: u32,
    ) -> Option<usize> {
        lines
            .iter()
            .enumerate()
            .take(setmanifest_line)
            .rev()
            .take_while(|(_, line)| Self::parse_setmanifest_line(line).is_none())
            .find_map(|(index, line)| {
                let (parsed_app_id, _) = Self::parse_addappid_line(line)?;
                (parsed_app_id == app_id).then_some(index)
            })
    }

    /// Rewrites one `setManifestid` line so its GID becomes `next_manifest_id`.
    ///
    /// Two shapes are accepted, because repair needs both:
    ///  * a well-formed call (quoted GID) — the original behaviour, which also
    ///    keeps a trailing third argument and any suffix;
    ///  * a malformed call — the whole argument list is replaced, keeping the
    ///    indentation, a possible `--` comment marker and the function spelling.
    ///
    /// When neither matches, the original arguments are intact-but-unusable
    /// (for example a non-numeric depot id) and the caller would otherwise have
    /// to guess a depot: refuse, so no partially rewritten line is written.
    fn rewrite_setmanifest_line(
        line: &str,
        next_manifest_id: &str,
    ) -> Result<String, String> {
        if let Some(caps) = SETMANIFEST_REWRITE_RE.captures(line) {
            return Ok(format!(
                "{}{}{}({}, \"{}\"){}",
                caps.name("indent").map(|m| m.as_str()).unwrap_or(""),
                caps.name("comment").map(|m| m.as_str()).unwrap_or(""),
                caps.name("func")
                    .map(|m| m.as_str())
                    .unwrap_or("setManifestid"),
                caps.name("appid").map(|m| m.as_str()).unwrap_or("0"),
                next_manifest_id,
                caps.name("suffix").map(|m| m.as_str()).unwrap_or(""),
            ));
        }

        // Fallback for a malformed call: keep the indentation, a `--` comment
        // marker, the function spelling and any trailing `--` note, but replace
        // the whole broken argument list (its GID may not even be quoted, so
        // there is nothing to substitute in place).
        let caps = SETMANIFEST_DEPOT_RE
            .captures(line)
            .ok_or_else(|| "Line is not a valid setManifestid call".to_string())?;
        let app_id = caps
            .name("appid")
            .map(|m| m.as_str())
            .unwrap_or("0")
            .to_string();
        let head_end = caps.get(0).map(|m| m.end()).unwrap_or(0);
        let head = line.get(..head_end).unwrap_or_default();
        let head_trimmed = head.trim_start();
        let indent = head.get(..head.len().saturating_sub(head_trimmed.len())).unwrap_or("");
        let comment = if head_trimmed.starts_with("--") {
            "-- "
        } else {
            ""
        };
        let function: String = head_trimmed
            .trim_start_matches('-')
            .trim_start()
            .split(['(', ' ', '\t'])
            .next()
            .unwrap_or_default()
            .to_string();
        let function = if function.is_empty() {
            "setManifestid".to_string()
        } else {
            function
        };
        let rest = line.get(head_end..).unwrap_or_default();
        let suffix = rest
            .find("--")
            .map(|comment_at| rest[comment_at..].trim_start())
            .unwrap_or("");

        Ok(match suffix.is_empty() {
            true => format!("{indent}{comment}{function}({app_id}, \"{next_manifest_id}\")"),
            false => {
                format!("{indent}{comment}{function}({app_id}, \"{next_manifest_id}\") {suffix}")
            }
        })
    }

    fn set_commented(line: &mut String, commented: bool) {
        let is_commented = line.trim_start().starts_with("--");
        match (commented, is_commented) {
            (true, false) => *line = format!("--{}", line),
            (false, true) => {
                let leading_len = line.len() - line.trim_start().len();
                let (leading, rest) = line.split_at(leading_len);
                let rest = rest.strip_prefix("--").unwrap_or(rest).trim_start();
                *line = format!("{}{}", leading, rest);
            }
            _ => {}
        }
    }

    /// Reads the raw Lua content with the editor's standard error context.
    /// Public read-only accessor: pipelines that need the exact bytes (pin
    /// realignment, backups, fingerprinting) must not re-derive the path.
    pub fn read_lua(&self) -> Result<String, String> {
        fs::read_to_string(&self.lua_path)
            .map_err(|e| format!("Failed to read {}: {}", self.lua_path.display(), e))
    }

    fn write_lua(&self, content: &str) -> Result<(), String> {
        let temp_path = self.lua_path.with_extension("tmp");
        fs::write(&temp_path, content)
            .map_err(|e| format!("Failed to write temporary Lua file: {}", e))?;
        fs::rename(&temp_path, &self.lua_path)
            .map_err(|e| format!("Failed to save Lua file: {}", e))
    }

    fn join_lua_lines(lines: &[String]) -> String {
        let mut content = lines.join("\n");
        content.push('\n');
        content
    }
}
