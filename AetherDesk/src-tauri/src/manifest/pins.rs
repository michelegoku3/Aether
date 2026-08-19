use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const HIDDEN_SYSTEM_DEPOTS: &[u32] = &[
    228981, 228982, 228983, 228984, 228985, 228986, 228987, 228988, 228989, 228990, 229000,
    229001, 229002, 229003, 229004, 229005, 229006, 229007, 229010, 229011, 229012, 229020,
    229030, 229031, 229032, 229033,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaManifestRow {
    /// Current line index of the setManifestid call in the Lua file.
    pub row_id: usize,
    pub app_id: u32,
    pub manifest_id: String,
    pub enabled: bool,
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

/// Outcome of `LuaManifestPins::apply_build_pins`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyBuildResult {
    /// Number of pins written into the Lua.
    pub applied_pins: usize,
    /// Kept for API compatibility; always empty with the current apply policy.
    /// A depot absent from a build's depot list is a depot that did NOT change
    /// in that patch, not a depot that was removed — so auto-apply never
    /// disables anything (the Manual editor remains the way to disable).
    pub disabled_depots: Vec<u32>,
}

#[derive(Debug, Clone)]
struct ManifestPin {
    row: LuaManifestRow,
    addappid_line: Option<usize>,
    addappid_enabled: bool,
    setmanifest_line: usize,
}

pub struct LuaManifestPins {
    lua_path: PathBuf,
}

impl LuaManifestPins {
    pub fn new(steam_path: impl Into<PathBuf>, root_app_id: u32) -> Self {
        Self {
            lua_path: steam_path
                .into()
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

    /// Copies the Lua to `<appid>.lua.bak` next to it. The backup is the
    /// restore point used by the version pipeline before any pin edit.
    pub fn backup(&self) -> Result<PathBuf, String> {
        if !self.path_exists() {
            return Err(format!(
                "Cannot back up {}: file does not exist",
                self.lua_path.display()
            ));
        }
        let backup_path = PathBuf::from(format!("{}.bak", self.lua_path.display()));
        fs::copy(&self.lua_path, &backup_path).map_err(|e| {
            format!(
                "Failed to back up {} to {}: {}",
                self.lua_path.display(),
                backup_path.display(),
                e
            )
        })?;
        Ok(backup_path)
    }

    /// Applies a build's pin diff to the Lua: every depot listed in `pins`
    /// gets its manifest pinned and is enabled; depots present in the Lua but
    /// absent from the build's depot list are left untouched (a build's list
    /// is a patch diff, so absence means the depot did not change in that
    /// build — never that it was removed). The row count is verified before
    /// saving — the same safety net as `apply_edits`.
    pub fn apply_build_pins(&self, pins: &[DepotManifestPin]) -> Result<ApplyBuildResult, String> {
        let content = self.read_lua()?;
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
                    // Depot changed in the target build: enable it and pin the manifest.
                    if let Some(addappid_line) = pin.addappid_line {
                        Self::set_commented(&mut lines[addappid_line], false);
                    }
                    Self::set_commented(&mut lines[pin.setmanifest_line], false);
                    if *manifest_id != pin.row.manifest_id {
                        lines[pin.setmanifest_line] = Self::rewrite_setmanifest_without_size(
                            &lines[pin.setmanifest_line],
                            manifest_id,
                        )?;
                    }
                    applied += 1;
                }
                None => {
                    // Depot absent from this build's depot list. A build's list
                    // is a patch diff: it only contains depots that CHANGED in
                    // that build, so an absent depot means "did not change", NOT
                    // "was removed" (e.g. the Windows/Linux/arch variants that
                    // often skip a patch). The existing pin stays enabled with
                    // its current manifest — Steam keeps serving that depot.
                    //
                    // Depots genuinely dropped from a game are handled in the
                    // Manual editor; auto-apply never disables anything.
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
            "Lua manifest {}: apply_build_pins completed -> {} pin(s) applied, {} depot(s) left unchanged (absent from the build diff)",
            self.lua_path.display(),
            applied,
            current.len() - applied
        );
        Ok(ApplyBuildResult {
            applied_pins: applied,
            disabled_depots: Vec::new(),
        })
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

    pub fn rows_from_file(&self) -> Result<Vec<LuaManifestRow>, String> {
        let content = self.read_lua()?;
        Ok(Self::rows_from_content(&content))
    }

    pub fn updates_are_enabled(&self) -> Result<bool, String> {
        let content = self.read_lua()?;
        let lines: Vec<&str> = content.lines().collect();

        // Updates are considered enabled only when at least one setManifestid pin
        // is commented while its related addappid is still enabled. If both addappid
        // and setManifestid are commented, that depot is disabled by the version
        // editor and must not affect the global Enable/Disable Update button.
        Ok(Self::pins_from_content(&content)
            .into_iter()
            .filter(|pin| pin.addappid_enabled)
            .any(|pin| lines
                .get(pin.setmanifest_line)
                .map(|line| line.trim_start().starts_with("--"))
                .unwrap_or(false)))
    }

    pub fn set_updates_enabled(&self, enabled: bool) -> Result<usize, String> {
        let content = self.read_lua()?;
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

            Self::set_commented(&mut lines[pin.setmanifest_line], enabled);
            changed += 1;
        }

        let next_content = Self::join_lua_lines(&lines);
        let after_count = Self::pins_from_content(&next_content).len();
        if after_count != pins.len() {
            return Err(format!(
                "Safety check failed: setManifestid count changed from {} to {}. File was not saved.",
                pins.len(), after_count
            ));
        }

        self.write_lua(&next_content)?;
        crate::desk_log_info!("manifest", "Lua manifest {}: set_updates_enabled({}) completed -> {} pin(s) modified", self.lua_path.display(), enabled, changed);
        Ok(changed)
    }

    pub fn apply_edits(&self, edits: Vec<LuaManifestEdit>) -> Result<Vec<LuaManifestRow>, String> {
        let content = self.read_lua()?;
        let pins = Self::pins_from_content(&content);
        let before_count = pins.len();
        let mut lines: Vec<String> = content.lines().map(str::to_string).collect();

        for edit in edits {
            let pin = pins
                .iter()
                .find(|pin| pin.row.row_id == edit.row_id)
                .ok_or_else(|| format!("setManifestid row {} was not found", edit.row_id))?;

            if let Some(addappid_line) = pin.addappid_line {
                Self::set_commented(&mut lines[addappid_line], !edit.enabled);
            }
            Self::set_commented(&mut lines[pin.setmanifest_line], !edit.enabled);

            if let Some(next_manifest_id) = edit.manifest_id.as_deref().map(str::trim) {
                if !next_manifest_id.is_empty() && next_manifest_id != pin.row.manifest_id {
                    lines[pin.setmanifest_line] = Self::rewrite_setmanifest_without_size(
                        &lines[pin.setmanifest_line],
                        next_manifest_id,
                    )?;
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
        let rows = self.rows_from_file()?;
        crate::desk_log_info!("manifest", "Lua manifest {}: apply_edits completed -> {} row(s) active", self.lua_path.display(), rows.len());
        Ok(rows)
    }

    fn pins_from_content(content: &str) -> Vec<ManifestPin> {
        let lines: Vec<&str> = content.lines().collect();
        let mut pins = Vec::new();

        for (line_index, line) in lines.iter().enumerate() {
            let Some((app_id, manifest_id, setmanifest_commented)) = Self::parse_setmanifest_line(line) else {
                continue;
            };
            let addappid_line = Self::find_nearest_addappid(&lines, line_index, app_id);
            let addappid_enabled = addappid_line
                .and_then(|index| Self::parse_addappid_line(lines[index]).map(|(_, commented)| !commented))
                .unwrap_or(!setmanifest_commented);

            pins.push(ManifestPin {
                row: LuaManifestRow {
                    row_id: line_index,
                    app_id,
                    manifest_id,
                    enabled: !setmanifest_commented && addappid_enabled,
                },
                addappid_line,
                addappid_enabled,
                setmanifest_line: line_index,
            });
        }

        pins
    }

    fn parse_setmanifest_line(line: &str) -> Option<(u32, String, bool)> {
        // Supports:
        // setManifestid(3764201, "299...", 123)
        // setmanifestid(3764201, '299...')
        // --setManifestid(...)
        // -- setManifestid(...)
        let re = Regex::new(
            r#"^\s*(?P<comment>--\s*)?(?i:setmanifestid)\s*\(\s*(?P<appid>\d+)\s*,\s*["'](?P<manifest>[^"']+)["']\s*(?:,[^)]*)?\)"#,
        ).ok()?;
        let caps = re.captures(line)?;
        Some((
            caps.name("appid")?.as_str().parse().ok()?,
            caps.name("manifest")?.as_str().to_string(),
            caps.name("comment").is_some(),
        ))
    }

    fn parse_addappid_line(line: &str) -> Option<(u32, bool)> {
        let re = Regex::new(
            r#"^\s*(?P<comment>--\s*)?(?i:addappid)\s*\(\s*(?P<appid>\d+)\b"#,
        ).ok()?;
        let caps = re.captures(line)?;
        Some((
            caps.name("appid")?.as_str().parse().ok()?,
            caps.name("comment").is_some(),
        ))
    }

    fn find_nearest_addappid(lines: &[&str], setmanifest_line: usize, app_id: u32) -> Option<usize> {
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

    fn rewrite_setmanifest_without_size(line: &str, next_manifest_id: &str) -> Result<String, String> {
        let re = Regex::new(
            r#"^(?P<indent>\s*)(?P<comment>--\s*)?(?P<func>(?i:setmanifestid))\s*\(\s*(?P<appid>\d+)\s*,\s*["'][^"']+["']\s*(?:,[^)]*)?\)(?P<suffix>.*)$"#,
        ).map_err(|e| format!("Internal regex error: {}", e))?;
        let caps = re
            .captures(line)
            .ok_or_else(|| "Line is not a valid setManifestid call".to_string())?;

        Ok(format!(
            "{}{}{}({}, \"{}\"){}",
            caps.name("indent").map(|m| m.as_str()).unwrap_or(""),
            caps.name("comment").map(|m| m.as_str()).unwrap_or(""),
            caps.name("func").map(|m| m.as_str()).unwrap_or("setManifestid"),
            caps.name("appid").map(|m| m.as_str()).unwrap_or("0"),
            next_manifest_id,
            caps.name("suffix").map(|m| m.as_str()).unwrap_or(""),
        ))
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

    fn read_lua(&self) -> Result<String, String> {
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
