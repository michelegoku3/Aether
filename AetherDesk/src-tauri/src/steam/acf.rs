use once_cell::sync::Lazy;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest::pins::DepotManifestPin;

/// Parses a VDF key line: `\t"buildid"\t\t"1234567"` → value "1234567".
/// VDF values are always double-quoted, so the regex is unambiguous.
static KEY_VALUE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^\s*"([^"]+)"\s+"([^"]*)""#).expect("static regex")
});

/// Rewrites only the value of a VDF key line, preserving indentation,
/// key casing and any trailing content.
static VALUE_REWRITE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^(\s*"[^"]+"\s*)"[^"]*"(.*)$"#).expect("static regex")
});

/// Owns minimal, safe edits of an existing `appmanifest_*.acf` file.
///
/// It only touches fields Steam itself manages when applying a downgrade:
/// `AppState.buildid`, `AppState.TargetBuildID` and
/// `InstalledDepots[depot].manifest` for depots already present. Everything
/// else — sizes, StateFlags, MountedDepots, AutoUpdateBehavior — is left
/// untouched (version pinning against Steam's updater is LumaCore's job).
/// Mirrors the behaviour of SFF's `_sync_acf_downgrade`, including the
/// atomic write, the read-only flag dance on Windows and a post-write
/// verification re-read.
pub struct SteamAcfEditor {
    path: PathBuf,
}

impl SteamAcfEditor {
    pub fn for_app(library_path: impl AsRef<Path>, app_id: u32) -> Self {
        Self {
            path: library_path
                .as_ref()
                .join("steamapps")
                .join(format!("appmanifest_{}.acf", app_id)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    pub fn read_raw(&self) -> Result<String, String> {
        fs::read_to_string(&self.path)
            .map_err(|e| format!("Failed to read {}: {}", self.path.display(), e))
    }

    /// Current `AppState.buildid` value, if present.
    pub fn build_id(&self) -> Result<String, String> {
        Self::find_value(&self.read_raw()?, "buildid")
    }

    /// `AppState.StateFlags` as a number (0 when unreadable).
    pub fn state_flags(&self) -> u64 {
        match self.read_raw() {
            Ok(content) => Self::find_value(&content, "StateFlags")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0),
            Err(_) => 0,
        }
    }

    pub fn is_readonly(&self) -> Result<bool, String> {
        let metadata = fs::metadata(&self.path)
            .map_err(|e| format!("Failed to stat {}: {}", self.path.display(), e))?;
        Ok(metadata.permissions().readonly())
    }

    /// Applies a build downgrade to the ACF. Returns an error when the file
    /// cannot be read/written (missing, locked by Steam, permissions); the
    /// caller decides whether to queue a retry.
    pub fn apply_build(&self, build_id: u64, pins: &[DepotManifestPin]) -> Result<(), String> {
        let content = self.read_raw()?;
        let lines: Vec<String> = content.lines().map(str::to_string).collect();
        let new_content = Self::rewrite_content(&lines, build_id, pins)?;

        let was_readonly = self.is_readonly()?;
        if was_readonly {
            let _ = self.set_readonly(false);
        }

        let write_result = (|| {
            let tmp = self.path.with_extension("acf.tmp");
            fs::write(&tmp, &new_content)
                .map_err(|e| format!("Failed to write {}: {}", tmp.display(), e))?;
            fs::rename(&tmp, &self.path)
                .map_err(|e| format!("Failed to replace {}: {}", self.path.display(), e))
        })();

        if was_readonly {
            let _ = self.set_readonly(true);
        }
        write_result?;

        // Verify the write actually stuck (file locked by Steam → fail here).
        let written = self.build_id()?;
        if written != build_id.to_string() {
            return Err(format!(
                "Verification failed: buildid is '{written}' instead of '{}' in {}",
                build_id,
                self.path.display()
            ));
        }
        Ok(())
    }

    fn set_readonly(&self, readonly: bool) -> Result<(), String> {
        let mut permissions = fs::metadata(&self.path)
            .map_err(|e| format!("Failed to stat {}: {}", self.path.display(), e))?
            .permissions();
        permissions.set_readonly(readonly);
        fs::set_permissions(&self.path, permissions)
            .map_err(|e| format!("Failed to update permissions on {}: {}", self.path.display(), e))
    }

    /// Line-based rewrite: only the touched lines change, everything else is
    /// preserved byte-for-byte.
    fn rewrite_content(
        lines: &[String],
        build_id: u64,
        pins: &[DepotManifestPin],
    ) -> Result<String, String> {
        let mut out = lines.to_vec();
        let build_id_str = build_id.to_string();

        // 1. AppState.buildid — replace, or insert after "appid" if missing.
        if let Some(index) = Self::find_key_line(&out, "buildid") {
            out[index] = Self::rewrite_value(&out[index], "buildid", &build_id_str)?;
        } else if let Some(appid_line) = Self::find_key_line(&out, "appid") {
            out.insert(appid_line + 1, format!("\t\"buildid\"\t\t\"{}\"", build_id_str));
        }

        // 2. AppState.TargetBuildID — replace, or insert after "buildid".
        if let Some(index) = Self::find_key_line(&out, "targetbuildid") {
            out[index] = Self::rewrite_value(&out[index], "targetbuildid", &build_id_str)?;
        } else if let Some(buildid_line) = Self::find_key_line(&out, "buildid") {
            out.insert(
                buildid_line + 1,
                format!("\t\"TargetBuildID\"\t\t\"{}\"", build_id_str),
            );
        }

        // 3. InstalledDepots[depot].manifest — only for depots already present.
        for pin in pins {
            let Some(depot_start) = Self::find_depot_block_start(&out, pin.depot_id) else {
                continue;
            };
            for index in depot_start + 1..out.len() {
                let trimmed = out[index].trim();
                if trimmed == "}" {
                    break; // end of this depot block — manifest line not found
                }
                if trimmed.starts_with("\"manifest\"") {
                    out[index] =
                        Self::rewrite_value(&out[index], "manifest", &pin.manifest_id)?;
                    break;
                }
            }
        }

        Ok(out.join("\n") + "\n")
    }

    /// Index of the first line starting with `"key"` followed by a value.
    fn find_key_line(lines: &[String], key: &str) -> Option<usize> {
        lines.iter().position(|line| {
            KEY_VALUE_RE
                .captures(line)
                .map(|caps| caps[1].eq_ignore_ascii_case(key))
                .unwrap_or(false)
        })
    }

    /// Index of the depot entry line: `\t"2347770"` alone on its line.
    fn find_depot_block_start(lines: &[String], depot_id: u32) -> Option<usize> {
        let needle = format!("\"{}\"", depot_id);
        lines.iter().position(|line| {
            let trimmed = line.trim();
            trimmed == needle && KEY_VALUE_RE.captures(line).is_none()
        })
    }

    /// Replaces only the value of a VDF key line, keeping formatting.
    fn rewrite_value(line: &str, key: &str, new_value: &str) -> Result<String, String> {
        let caps = VALUE_REWRITE_RE
            .captures(line)
            .ok_or_else(|| format!("Line is not a valid VDF entry: {line:?}"))?;
        // Guard: never rewrite a different key than requested.
        let key_start = line.trim_start().trim_start_matches('"');
        if !key_start.to_ascii_lowercase().starts_with(&key.to_ascii_lowercase()) {
            return Err(format!(
                "VDF key mismatch: expected '{key}' in line {line:?}"
            ));
        }
        Ok(format!("{}\"{}\"{}", &caps[1], new_value, &caps[2]))
    }

    /// First `"key" "value"` pair in the raw content.
    fn find_value(content: &str, key: &str) -> Result<String, String> {
        for line in content.lines() {
            if let Some(caps) = KEY_VALUE_RE.captures(line) {
                if caps[1].eq_ignore_ascii_case(key) {
                    return Ok(caps[2].to_string());
                }
            }
        }
        Err(format!(
            "Key '{key}' not found in {}",
            content.lines().count()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACF: &str = "\"AppState\"\n{\n\t\"appid\"\t\t\"730\"\n\t\"Universe\"\t\t\"1\"\n\t\"name\"\t\t\"Counter-Strike 2\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"Counter-Strike Global Offensive\"\n\t\"LastUpdated\"\t\t\"1723456789\"\n\t\"UpdateResult\"\t\t\"0\"\n\t\"SizeOnDisk\"\t\t\"38173519462\"\n\t\"buildid\"\t\t\"24537688\"\n\t\"TargetBuildID\"\t\t\"24537688\"\n\t\"LastOwner\"\t\t\"76561198000000000\"\n\t\"BytesToDownload\"\t\t\"0\"\n\t\"BytesDownloaded\"\t\t\"0\"\n\t\"AutoUpdateBehavior\"\t\t\"1\"\n\t\"AllowOtherDownloadsWhileRunning\"\t\t\"0\"\n\t\"ScheduledAutoUpdate\"\t\t\"0\"\n\t\"InstalledDepots\"\n\t{\n\t\t\"2347770\"\n\t\t{\n\t\t\t\"manifest\"\t\t\"2991528520052157173\"\n\t\t\t\"size\"\t\t\"38173519462\"\n\t\t}\n\t\t\"2347771\"\n\t\t{\n\t\t\t\"manifest\"\t\t\"8124921270987929782\"\n\t\t\t\"size\"\t\t\"0\"\n\t\t\t\"dlcappid\"\t\t\"0\"\n\t\t}\n\t}\n\t\"MountedDepots\"\n\t{\n\t\t\"2347770\"\t\t\"2991528520052157173\"\n\t}\n}\n";

    #[test]
    fn applies_build_and_keeps_everything_else() {
        let lines: Vec<String> = ACF.lines().map(str::to_string).collect();
        let out = SteamAcfEditor::rewrite_content(
            &lines,
            24701871,
            &[DepotManifestPin {
                depot_id: 2347770,
                manifest_id: "1111111111111111111".to_string(),
            }],
        )
        .unwrap();

        assert!(out.contains("\"buildid\"\t\t\"24701871\""));
        assert!(out.contains("\"TargetBuildID\"\t\t\"24701871\""));
        // Pinned depot updated…
        assert!(out.contains("\"manifest\"\t\t\"1111111111111111111\""));
        // …depot not in pins untouched…
        assert!(out.contains("\"manifest\"\t\t\"8124921270987929782\""));
        // …and unrelated fields preserved verbatim.
        assert!(out.contains("\"StateFlags\"\t\t\"4\""));
        assert!(out.contains("\"AutoUpdateBehavior\"\t\t\"1\""));
        assert!(out.contains("\"size\"\t\t\"38173519462\""));
        assert!(out.contains("\"MountedDepots\""));
    }

    #[test]
    fn inserts_missing_buildid_and_target() {
        let minimal = "\"AppState\"\n{\n\t\"appid\"\t\t\"730\"\n\t\"StateFlags\"\t\t\"0\"\n}\n";
        let lines: Vec<String> = minimal.lines().map(str::to_string).collect();
        let out = SteamAcfEditor::rewrite_content(&lines, 99, &[]).unwrap();
        assert!(out.contains("\"buildid\"\t\t\"99\""));
        assert!(out.contains("\"TargetBuildID\"\t\t\"99\""));
        // appid line still first
        let appid_pos = out.find("\"appid\"").unwrap();
        let bid_pos = out.find("\"buildid\"").unwrap();
        assert!(appid_pos < bid_pos);
    }

    #[test]
    fn ignores_pins_for_unknown_depots() {
        let lines: Vec<String> = ACF.lines().map(str::to_string).collect();
        let out = SteamAcfEditor::rewrite_content(
            &lines,
            1,
            &[DepotManifestPin {
                depot_id: 9999999,
                manifest_id: "42".to_string(),
            }],
        )
        .unwrap();
        assert!(!out.contains("\"9999999\""));
    }

    #[test]
    fn depot_block_start_requires_standalone_key() {
        let lines = vec![
            "\t\"2347770\"".to_string(),
            "\t\"manifest\"\t\t\"1\"".to_string(),
            "\t\"size\"\t\t\"0\"".to_string(),
        ];
        assert_eq!(SteamAcfEditor::find_depot_block_start(&lines, 2347770), Some(0));
        // A key-value line with the same digits is not a depot block start.
        let kv = vec!["\t\"2347770\"\t\t\"123\"".to_string()];
        assert_eq!(SteamAcfEditor::find_depot_block_start(&kv, 2347770), None);
    }
}
