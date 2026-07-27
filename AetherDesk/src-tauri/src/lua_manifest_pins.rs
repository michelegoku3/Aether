use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const HIDDEN_SYSTEM_DEPOTS: &[u32] = &[228986, 228989, 228990];

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
        self.rows_from_file()
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
