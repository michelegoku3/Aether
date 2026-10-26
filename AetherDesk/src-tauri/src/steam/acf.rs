use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::manifest::identity::manifest_content_size;
use crate::steam::library::SteamLibraryScanner;

/// Parses a VDF key line: `\t"buildid"\t\t"1234567"` → value "1234567".
/// VDF values are always double-quoted, so the regex is unambiguous.
static KEY_VALUE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*"([^"]+)"\s+"([^"]*)""#).expect("static regex"));

/// Rewrites only the value of a VDF key line, preserving indentation,
/// key casing and any trailing content.
static VALUE_REWRITE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^(\s*"[^"]+"\s*)"[^"]*"(.*)$"#).expect("static regex"));

/// One `InstalledDepots[depot]` entry plus the line numbers of the values a
/// repair may rewrite.
struct AcfDepotEntry {
    depot_id: u32,
    manifest_line: usize,
    manifest_gid: String,
    size_line: Option<usize>,
    size: Option<u64>,
}

/// One local `depotcache` file that can back an `InstalledDepots` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalManifestCandidate {
    pub gid: String,
    /// Sum of the manifest's file sizes, when the file parses.
    pub content_size: Option<u64>,
    pub modified: Option<SystemTime>,
}

/// One `InstalledDepots[depot].manifest` GID that no local manifest backed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledDepotRepair {
    pub depot_id: u32,
    pub declared_gid: String,
    pub restored_gid: String,
}

/// Owns minimal, safe edits of an existing `appmanifest_*.acf` file.
///
/// It only touches fields Steam itself manages when applying a downgrade:
/// `AppState.buildid` and `AppState.TargetBuildID`. Everything else — sizes,
/// StateFlags, InstalledDepots, MountedDepots, AutoUpdateBehavior — is left
/// untouched (version pinning against Steam's updater is LumaCore's job).
///
/// `InstalledDepots` in particular is never written from a pin: it records
/// which manifest Steam actually installed, and Steam computes the update
/// delta against exactly that GID. Writing a pinned GID there before its
/// content is installed claims an install Steam cannot read, so the depot's
/// manifest request fails forever (`Access Denied` from the CM, "No
/// connection" in the UI). When that mismatch already exists,
/// [`Self::repair_installed_depots`] restores the GID of the manifest on disk.
/// Mirrors the behaviour of SFF's `_sync_acf_downgrade` for the build fields,
/// including the atomic write, the read-only flag dance on Windows and a
/// post-write verification re-read.
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

    pub fn read_raw(&self) -> Result<String, String> {
        fs::read_to_string(&self.path)
            .map_err(|e| format!("Failed to read {}: {}", self.path.display(), e))
    }

    /// Current `AppState.buildid` value, if present.
    pub fn build_id(&self) -> Result<String, String> {
        Self::find_value(&self.read_raw()?, "buildid")
    }

    /// `InstalledDepots[depot_id].manifest`: the manifest GID Steam declares
    /// installed for one depot. `None` when the file is unreadable, the depot
    /// is not listed or the value is not a positive GID. This is Steam's own
    /// statement of what is on disk — the authoritative answer to "which
    /// version does the user actually have", independent of file mtimes in
    /// depotcache (which restores, stagings and abandoned updates can skew).
    pub fn installed_depot_manifest(&self, depot_id: u32) -> Option<String> {
        let content = self.read_raw().ok()?;
        let lines: Vec<String> = content.lines().map(str::to_string).collect();
        Self::installed_depot_manifest_in(&lines, depot_id)
    }

    fn installed_depot_manifest_in(lines: &[String], depot_id: u32) -> Option<String> {
        Self::installed_depot_entries(lines)
            .into_iter()
            .find(|entry| entry.depot_id == depot_id)
            .map(|entry| entry.manifest_gid)
            .filter(|gid| gid.parse::<u64>().map(|gid| gid > 0).unwrap_or(false))
    }

    /// Every depot entry inside `InstalledDepots`, with the line numbers of the
    /// `manifest`/`size` values so a repair can rewrite them in place.
    fn installed_depot_entries(lines: &[String]) -> Vec<AcfDepotEntry> {
        let Some((block_start, block_end)) = Self::installed_depots_block(lines) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        let mut index = block_start + 1;
        while index < block_end {
            let Some(depot_id) = Self::standalone_depot_id(&lines[index]) else {
                index += 1;
                continue;
            };
            let mut cursor = index + 1;
            while cursor < block_end && lines[cursor].trim().is_empty() {
                cursor += 1;
            }
            if cursor >= block_end || lines[cursor].trim() != "{" {
                index += 1;
                continue;
            }
            let mut depth = 0i32;
            let mut manifest: Option<(usize, String)> = None;
            let mut size: Option<(usize, Option<u64>)> = None;
            let mut close = cursor;
            for (fill, line) in lines.iter().enumerate().take(block_end).skip(cursor) {
                let trimmed = line.trim();
                if trimmed == "{" {
                    depth += 1;
                    continue;
                }
                if trimmed == "}" {
                    depth -= 1;
                    if depth == 0 {
                        close = fill;
                        break;
                    }
                    continue;
                }
                let Some(caps) = KEY_VALUE_RE.captures(line) else {
                    continue;
                };
                if caps[1].eq_ignore_ascii_case("manifest") {
                    manifest = Some((fill, caps[2].trim().to_string()));
                }
                if caps[1].eq_ignore_ascii_case("size") {
                    size = Some((fill, caps[2].trim().parse::<u64>().ok()));
                }
            }
            if let Some((manifest_line, manifest_gid)) = manifest {
                let (size_line, size_value) = size.unwrap_or((0, None));
                entries.push(AcfDepotEntry {
                    depot_id,
                    manifest_line,
                    manifest_gid,
                    size_line: (size_line > 0).then_some(size_line),
                    size: size_value,
                });
            }
            index = close + 1;
        }
        entries
    }

    /// Line span of the `InstalledDepots` block: the key line and its closing
    /// brace. Scoped so a bare `"<depot>"` inside another block never matches
    /// (e.g. `SharedDepots` opens sub-blocks the same way).
    fn installed_depots_block(lines: &[String]) -> Option<(usize, usize)> {
        let start = lines
            .iter()
            .position(|line| line.trim().eq_ignore_ascii_case("\"InstalledDepots\""))?;
        let mut depth = 0i32;
        let mut opened = false;
        for (index, line) in lines.iter().enumerate().skip(start + 1) {
            match line.trim() {
                "{" => {
                    depth += 1;
                    opened = true;
                }
                "}" => {
                    depth -= 1;
                    if opened && depth <= 0 {
                        return Some((start, index));
                    }
                }
                _ => {}
            }
        }
        Some((start, lines.len()))
    }

    /// A standalone `\t"<depot>"` line (no value after it).
    fn standalone_depot_id(line: &str) -> Option<u32> {
        if KEY_VALUE_RE.captures(line).is_some() {
            return None;
        }
        line.trim()
            .strip_prefix('"')?
            .strip_suffix('"')?
            .parse::<u32>()
            .ok()
    }

    /// Restores every `InstalledDepots` entry whose declared GID has no local
    /// manifest file. Steam asks the CM for that exact GID before it can start
    /// a download or an update; when no file backs it the request is denied and
    /// the whole download dies. `depot_dirs` are the `depotcache` folders to
    /// look in (`<steam>/depotcache` and `<steam>/config/depotcache`).
    ///
    /// Selection: the manifest whose own content size equals the size Steam
    /// recorded for the depot wins — that is the file the installed content
    /// came from. Otherwise the newest local manifest for that depot is used:
    /// Steam re-verifies every chunk anyway, so a harmless extra verification
    /// beats a GID that cannot be fetched at all.
    pub fn repair_installed_depots(
        &self,
        depot_dirs: &[PathBuf],
    ) -> Result<Vec<InstalledDepotRepair>, String> {
        let content = self.read_raw()?;
        let lines: Vec<String> = content.lines().map(str::to_string).collect();
        let (repaired_lines, repairs) = Self::repair_installed_depots_in(
            &lines,
            |depot_id, gid| Self::manifest_on_disk(depot_dirs, depot_id, gid),
            |depot_id| Self::local_manifest_candidates(depot_dirs, depot_id),
        )?;
        if repairs.is_empty() {
            return Ok(repairs);
        }
        self.write_content(&content, &(repaired_lines.join("\n") + "\n"))?;
        Ok(repairs)
    }

    /// True when `<depot>_<gid>.manifest` is a non-empty file in one of the
    /// cache directories. Cheap existence probe: the sweep runs this for every
    /// managed depot, and the common answer is "already fine".
    fn manifest_on_disk(directories: &[PathBuf], depot_id: u32, gid: &str) -> bool {
        let name = format!("{depot_id}_{gid}.manifest");
        directories.iter().any(|directory| {
            fs::metadata(directory.join(&name))
                .map(|metadata| metadata.len() > 0)
                .unwrap_or(false)
        })
    }

    /// Pure half of [`Self::repair_installed_depots`]: both filesystem probes
    /// are callbacks so the decision matrix is testable without a directory.
    fn repair_installed_depots_in(
        lines: &[String],
        mut is_backed: impl FnMut(u32, &str) -> bool,
        mut candidates: impl FnMut(u32) -> Vec<LocalManifestCandidate>,
    ) -> Result<(Vec<String>, Vec<InstalledDepotRepair>), String> {
        let mut out = lines.to_vec();
        let mut repairs = Vec::new();
        for entry in Self::installed_depot_entries(lines) {
            if entry.manifest_gid.parse::<u64>().unwrap_or(0) == 0 {
                continue;
            }
            if is_backed(entry.depot_id, &entry.manifest_gid) {
                continue;
            }
            let available = candidates(entry.depot_id);
            if available
                .iter()
                .any(|candidate| candidate.gid == entry.manifest_gid)
            {
                continue;
            }
            let Some(chosen) = Self::select_local_manifest(&entry, &available) else {
                continue;
            };
            out[entry.manifest_line] =
                Self::rewrite_value(&out[entry.manifest_line], "manifest", &chosen.gid)?;
            if let (Some(size_line), Some(content_size)) = (entry.size_line, chosen.content_size) {
                out[size_line] =
                    Self::rewrite_value(&out[size_line], "size", &content_size.to_string())?;
            }
            repairs.push(InstalledDepotRepair {
                depot_id: entry.depot_id,
                declared_gid: entry.manifest_gid.clone(),
                restored_gid: chosen.gid,
            });
        }
        Ok((out, repairs))
    }

    fn select_local_manifest(
        entry: &AcfDepotEntry,
        candidates: &[LocalManifestCandidate],
    ) -> Option<LocalManifestCandidate> {
        if let Some(size) = entry.size.filter(|size| *size > 0) {
            if let Some(hit) = candidates
                .iter()
                .find(|candidate| candidate.content_size == Some(size))
            {
                return Some(hit.clone());
            }
        }
        candidates
            .iter()
            .max_by(|left, right| left.modified.cmp(&right.modified))
            .cloned()
    }

    fn local_manifest_candidates(
        directories: &[PathBuf],
        depot_id: u32,
    ) -> Vec<LocalManifestCandidate> {
        let prefix = format!("{depot_id}_");
        let mut found = Vec::new();
        for directory in directories {
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                let Some(gid) = name
                    .strip_prefix(&prefix)
                    .and_then(|stem| stem.strip_suffix(".manifest"))
                    .filter(|gid| gid.parse::<u64>().map(|gid| gid > 0).unwrap_or(false))
                else {
                    continue;
                };
                let Ok(metadata) = fs::metadata(&path) else {
                    continue;
                };
                if metadata.len() == 0 {
                    continue;
                }
                let content_size = fs::read(&path)
                    .ok()
                    .and_then(|bytes| manifest_content_size(&bytes).ok());
                found.push(LocalManifestCandidate {
                    gid: gid.to_string(),
                    content_size,
                    modified: metadata.modified().ok(),
                });
            }
        }
        found
    }

    /// Repairs `app_id`'s ACF in whichever library declares it.
    pub fn repair_installed_depots_for_app(
        steam_path: &str,
        app_id: u32,
    ) -> Result<Vec<InstalledDepotRepair>, String> {
        let directories = Self::depot_cache_dirs(steam_path);
        let mut repairs = Vec::new();
        for library in SteamLibraryScanner::new(steam_path).discover_library_paths() {
            let editor = Self::for_app(&library, app_id);
            if !editor.path.exists() {
                continue;
            }
            repairs.extend(editor.repair_installed_depots(&directories)?);
        }
        Ok(repairs)
    }

    fn depot_cache_dirs(steam_path: &str) -> Vec<PathBuf> {
        let root = PathBuf::from(steam_path);
        vec![
            root.join("depotcache"),
            root.join("config").join("depotcache"),
        ]
    }

    pub fn is_readonly(&self) -> Result<bool, String> {
        let metadata = fs::metadata(&self.path)
            .map_err(|e| format!("Failed to stat {}: {}", self.path.display(), e))?;
        Ok(metadata.permissions().readonly())
    }

    /// Applies a build downgrade to the ACF. Returns an error when the file
    /// cannot be read/written (missing, locked by Steam, permissions); the
    /// caller logs a WARN and continues (best-effort metadata sync).
    pub fn apply_build(&self, build_id: u64) -> Result<(), String> {
        let content = self.read_raw()?;
        let lines: Vec<String> = content.lines().map(str::to_string).collect();
        let new_content = Self::rewrite_content(&lines, build_id)?;
        self.write_content(&content, &new_content)?;

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

    /// Atomic write with the read-only dance Steam leaves on some libraries.
    fn write_content(&self, current: &str, next: &str) -> Result<(), String> {
        let was_readonly = self.is_readonly()?;
        if was_readonly {
            let _ = self.set_readonly(false);
        }

        let write_result = crate::core::state_io::write_if_unchanged(
            &self.path,
            current.as_bytes(),
            next.as_bytes(),
        );

        if was_readonly {
            let _ = self.set_readonly(true);
        }
        write_result
    }

    fn set_readonly(&self, readonly: bool) -> Result<(), String> {
        let mut permissions = fs::metadata(&self.path)
            .map_err(|e| format!("Failed to stat {}: {}", self.path.display(), e))?
            .permissions();
        permissions.set_readonly(readonly);
        fs::set_permissions(&self.path, permissions).map_err(|e| {
            format!(
                "Failed to update permissions on {}: {}",
                self.path.display(),
                e
            )
        })
    }

    /// Line-based rewrite: only the build lines change, everything else is
    /// preserved byte-for-byte.
    fn rewrite_content(lines: &[String], build_id: u64) -> Result<String, String> {
        let mut out = lines.to_vec();
        let build_id_str = build_id.to_string();

        // 1. AppState.buildid — replace, or insert after "appid" if missing.
        if let Some(index) = Self::find_key_line(&out, "buildid") {
            out[index] = Self::rewrite_value(&out[index], "buildid", &build_id_str)?;
        } else if let Some(appid_line) = Self::find_key_line(&out, "appid") {
            out.insert(
                appid_line + 1,
                format!("\t\"buildid\"\t\t\"{}\"", build_id_str),
            );
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

    /// Replaces only the value of a VDF key line, keeping formatting.
    fn rewrite_value(line: &str, key: &str, new_value: &str) -> Result<String, String> {
        let caps = VALUE_REWRITE_RE
            .captures(line)
            .ok_or_else(|| format!("Line is not a valid VDF entry: {line:?}"))?;
        // Guard: never rewrite a different key than requested.
        let key_start = line.trim_start().trim_start_matches('"');
        if !key_start
            .to_ascii_lowercase()
            .starts_with(&key.to_ascii_lowercase())
        {
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
        let out = SteamAcfEditor::rewrite_content(&lines, 24701871).unwrap();

        assert!(out.contains("\"buildid\"\t\t\"24701871\""));
        assert!(out.contains("\"TargetBuildID\"\t\t\"24701871\""));
        // InstalledDepots is Steam's record of what is on disk: a build pin
        // must never claim a manifest whose content was not installed.
        assert!(out.contains("\"manifest\"\t\t\"2991528520052157173\""));
        assert!(out.contains("\"manifest\"\t\t\"8124921270987929782\""));
        // …and unrelated fields preserved verbatim.
        assert!(out.contains("\"StateFlags\"\t\t\"4\""));
        assert!(out.contains("\"AutoUpdateBehavior\"\t\t\"1\""));
        assert!(out.contains("\"size\"\t\t\"38173519462\""));
        assert!(out.contains("\"MountedDepots\""));
    }

    #[test]
    fn reads_installed_depot_manifests_from_the_installed_depots_block() {
        let lines: Vec<String> = ACF.lines().map(str::to_string).collect();
        assert_eq!(
            SteamAcfEditor::installed_depot_manifest_in(&lines, 2347770).as_deref(),
            Some("2991528520052157173")
        );
        assert_eq!(
            SteamAcfEditor::installed_depot_manifest_in(&lines, 2347771).as_deref(),
            Some("8124921270987929782")
        );
        // Unknown depot → None; a "0" manifest never counts as installed.
        assert_eq!(
            SteamAcfEditor::installed_depot_manifest_in(&lines, 42),
            None
        );
        let zero = ACF.replace("\"8124921270987929782\"", "\"0\"");
        let lines: Vec<String> = zero.lines().map(str::to_string).collect();
        assert_eq!(
            SteamAcfEditor::installed_depot_manifest_in(&lines, 2347771),
            None
        );
        // No InstalledDepots block at all → None.
        let minimal = "\"AppState\"\n{\n\t\"appid\"\t\t\"730\"\n}\n";
        let lines: Vec<String> = minimal.lines().map(str::to_string).collect();
        assert_eq!(
            SteamAcfEditor::installed_depot_manifest_in(&lines, 2347770),
            None
        );
    }

    #[test]
    fn inserts_missing_buildid_and_target() {
        let minimal = "\"AppState\"\n{\n\t\"appid\"\t\t\"730\"\n\t\"StateFlags\"\t\t\"0\"\n}\n";
        let lines: Vec<String> = minimal.lines().map(str::to_string).collect();
        let out = SteamAcfEditor::rewrite_content(&lines, 99).unwrap();
        assert!(out.contains("\"buildid\"\t\t\"99\""));
        assert!(out.contains("\"TargetBuildID\"\t\t\"99\""));
        // appid line still first
        let appid_pos = out.find("\"appid\"").unwrap();
        let bid_pos = out.find("\"buildid\"").unwrap();
        assert!(appid_pos < bid_pos);
    }

    /// One depot whose declared GID is `532` and whose recorded size is `100`.
    const PHANTOM_ACF: &str = "\"AppState\"\n{\n\t\"appid\"\t\t\"4656000\"\n\t\"buildid\"\t\t\"25904038\"\n\t\"TargetBuildID\"\t\t\"26188740\"\n\t\"InstalledDepots\"\n\t{\n\t\t\"4656002\"\n\t\t{\n\t\t\t\"manifest\"\t\t\"532\"\n\t\t\t\"size\"\t\t\"100\"\n\t\t}\n\t}\n}\n";

    fn candidate(
        gid: &str,
        content_size: Option<u64>,
        modified_secs: u64,
    ) -> LocalManifestCandidate {
        LocalManifestCandidate {
            gid: gid.to_string(),
            content_size,
            modified: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified_secs)),
        }
    }

    #[test]
    fn repairs_a_phantom_installed_gid_with_the_size_match() {
        let lines: Vec<String> = PHANTOM_ACF.lines().map(str::to_string).collect();
        let (out, repairs) = SteamAcfEditor::repair_installed_depots_in(
            &lines,
            |_, _| false,
            |depot_id| {
                assert_eq!(depot_id, 4656002);
                vec![
                    // Newer, but it describes a different (target) build.
                    candidate("4298777078523366725", Some(7), 20),
                    // What Steam actually installed: same size as the ACF.
                    candidate("5722240766019054112", Some(100), 10),
                ]
            },
        )
        .unwrap();

        assert_eq!(
            repairs,
            vec![InstalledDepotRepair {
                depot_id: 4656002,
                declared_gid: "532".to_string(),
                restored_gid: "5722240766019054112".to_string(),
            }]
        );
        let joined = out.join("\n");
        assert!(joined.contains("\"manifest\"\t\t\"5722240766019054112\""));
        assert!(joined.contains("\"size\"\t\t\"100\""));
        // Build fields are Steam's to update; a repair touches depots only.
        assert!(joined.contains("\"buildid\"\t\t\"25904038\""));
        assert!(joined.contains("\"TargetBuildID\"\t\t\"26188740\""));
    }

    #[test]
    fn repairs_with_the_newest_manifest_when_no_size_matches() {
        let lines: Vec<String> = PHANTOM_ACF.lines().map(str::to_string).collect();
        let (out, repairs) = SteamAcfEditor::repair_installed_depots_in(
            &lines,
            |_, _| false,
            |_| {
                vec![
                    candidate("11", Some(1), 10),
                    candidate("22", None, 30),
                    candidate("33", Some(2), 20),
                ]
            },
        )
        .unwrap();

        assert_eq!(repairs.len(), 1);
        assert_eq!(repairs[0].restored_gid, "22");
        // The chosen manifest carries no parsable size: leave Steam's number.
        assert!(out.join("\n").contains("\"size\"\t\t\"100\""));
    }

    #[test]
    fn leaves_a_resolvable_installed_gid_alone() {
        let lines: Vec<String> = PHANTOM_ACF.lines().map(str::to_string).collect();
        // The probe finds the declared GID on disk: no candidate scan at all.
        let (out, repairs) = SteamAcfEditor::repair_installed_depots_in(
            &lines,
            |_, gid| gid == "532",
            |_| panic!("a backed depot must not be scanned"),
        )
        .unwrap();

        assert!(repairs.is_empty());
        assert_eq!(out, lines);
    }

    #[test]
    fn reports_a_phantom_gid_that_no_local_manifest_can_back() {
        let lines: Vec<String> = PHANTOM_ACF.lines().map(str::to_string).collect();
        let (out, repairs) =
            SteamAcfEditor::repair_installed_depots_in(&lines, |_, _| false, |_| Vec::new())
                .unwrap();

        assert!(repairs.is_empty());
        assert_eq!(out, lines);
    }

    #[test]
    fn repair_round_trips_through_the_acf_file() {
        let directory = tempfile::tempdir().unwrap();
        let steam = directory.path();
        let depotcache = steam.join("depotcache");
        std::fs::create_dir_all(&depotcache).unwrap();
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        // Two unparsable candidates: the decision falls back to newest, which is
        // enough to prove the file write path (read-only dance, atomic write).
        std::fs::write(depotcache.join("4656002_1111.manifest"), b"old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(depotcache.join("4656002_2222.manifest"), b"new").unwrap();
        let acf = steam.join("steamapps").join("appmanifest_4656000.acf");
        std::fs::write(&acf, PHANTOM_ACF).unwrap();

        let repairs = SteamAcfEditor::for_app(steam, 4656000)
            .repair_installed_depots(std::slice::from_ref(&depotcache))
            .unwrap();

        assert_eq!(repairs.len(), 1);
        assert_eq!(repairs[0].restored_gid, "2222");
        let written = std::fs::read_to_string(&acf).unwrap();
        assert!(written.contains("\"manifest\"\t\t\"2222\""));
    }

    #[test]
    fn depot_block_start_requires_standalone_key() {
        // A bare `"<depot>"` line opens a depot block…
        assert_eq!(
            SteamAcfEditor::standalone_depot_id("\t\"2347770\""),
            Some(2347770)
        );
        // …while a key-value line with the same digits is not one.
        assert_eq!(
            SteamAcfEditor::standalone_depot_id("\t\"2347770\"\t\t\"123\""),
            None
        );
        assert_eq!(SteamAcfEditor::standalone_depot_id("\t\"manifest\""), None);
    }
}
