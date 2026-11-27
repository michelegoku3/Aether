use std::fs;
use std::io::Write;
use std::path::PathBuf;

const STEAM_CFG_FILE_NAME: &str = "steam.cfg";
/// Sibling the block write goes through before the rename: a crash mid-write
/// must never truncate the live `steam.cfg`.
const TEMP_FILE_SUFFIX: &str = ".aether-tmp";
const UPDATE_BLOCK_KEY: &str = "BootStrapperInhibitAll";
const BLOCKED_VALUE: &str = "Enable";

/// Owns every read/write/delete operation related to Steam's update guard config.
///
/// The UI and Tauri commands should not know how `steam.cfg` is structured:
/// they only ask whether updates are blocked or request a state change.
/// This keeps file parsing/writing isolated and easier to maintain.
///
/// Only the blocked state needs a file: Steam reads `steam.cfg` at boot, so the
/// directive is written through a temp file and renamed (a partial write can
/// never leave a truncated config behind). Re-enabling updates **deletes** the
/// file instead of writing `Disable`: absence is the state Steam treats as "no
/// block", and a file that does not exist cannot be malformed.
pub struct SteamUpdateGuard {
    steam_dir: PathBuf,
}

impl SteamUpdateGuard {
    pub fn new(steam_path: impl Into<PathBuf>) -> Self {
        // Normalize at the boundary so quoted/padded-but-valid paths work.
        let normalized =
            crate::steam::resolve::normalize_steam_path(&steam_path.into().to_string_lossy());
        Self {
            steam_dir: PathBuf::from(normalized),
        }
    }

    pub fn config_path(&self) -> PathBuf {
        self.steam_dir.join(STEAM_CFG_FILE_NAME)
    }

    /// Returns the persisted state from steam.cfg. If the file does not exist,
    /// updates are considered unblocked and the file is not created.
    pub fn is_blocked(&self) -> Result<bool, String> {
        self.validate_steam_dir()?;
        if !self.config_path().exists() {
            return Ok(false);
        }

        let content = self.read_config()?;

        Ok(content.lines().any(|line| {
            let Some((key, value)) = Self::parse_key_value(line) else {
                return false;
            };

            key.eq_ignore_ascii_case(UPDATE_BLOCK_KEY) && value.eq_ignore_ascii_case(BLOCKED_VALUE)
        }))
    }

    pub fn block_updates(&self) -> Result<(), String> {
        self.validate_steam_dir()?;

        let cfg_path = self.config_path();
        let content = if cfg_path.exists() {
            self.read_config()?
        } else {
            String::new()
        };

        self.write_config_atomically(&self.upsert_update_directive(&content))
    }

    /// Re-enables Steam client updates by removing `steam.cfg`.
    ///
    /// Deleting is the one transition that cannot corrupt the guard: Steam
    /// treats an absent file as "not blocked", while a rewritten one can be
    /// truncated or carry a directive its bootstrapper rejects. Absent file is
    /// therefore a success, not an error (idempotent unlock).
    pub fn unblock_updates(&self) -> Result<(), String> {
        self.validate_steam_dir()?;

        let cfg_path = self.config_path();
        if !cfg_path.exists() {
            return Ok(());
        }

        fs::remove_file(&cfg_path).map_err(|e| format!("Failed to delete steam.cfg: {}", e))
    }

    /// Writes the whole config through a sibling temp file, then renames it over
    /// `steam.cfg`. `fs::rename` replaces the destination atomically on the same
    /// volume, so Steam (which reads the file at boot) never observes a partial
    /// write.
    fn write_config_atomically(&self, content: &str) -> Result<(), String> {
        let cfg_path = self.config_path();
        let temp_path = self.temp_config_path();

        let written = fs::File::create(&temp_path).and_then(|mut file| {
            file.write_all(content.as_bytes())?;
            file.sync_all()
        });
        if let Err(error) = written {
            let _ = fs::remove_file(&temp_path);
            return Err(format!("Failed to update steam.cfg: {}", error));
        }

        if let Err(error) = fs::rename(&temp_path, &cfg_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!("Failed to update steam.cfg: {}", error));
        }

        Ok(())
    }

    fn temp_config_path(&self) -> PathBuf {
        let mut name = self.config_path().into_os_string();
        name.push(TEMP_FILE_SUFFIX);
        PathBuf::from(name)
    }

    fn validate_steam_dir(&self) -> Result<(), String> {
        // Full validation (directory + steam.exe): writing steam.cfg into a
        // typo'd-but-existing folder would be worse than failing loudly.
        let raw = self.steam_dir.to_string_lossy();
        crate::steam::resolve::resolve_steam_path(&raw)
            .map(|_| ())
            .map_err(|error| error.message(&raw))
    }

    fn read_config(&self) -> Result<String, String> {
        fs::read_to_string(self.config_path())
            .map_err(|e| format!("Failed to read steam.cfg: {}", e))
    }

    fn upsert_update_directive(&self, content: &str) -> String {
        let directive = self.render_directive().trim_end().to_string();
        let mut found = false;

        let mut lines: Vec<String> = content
            .lines()
            .map(|line| {
                let Some((key, _)) = Self::parse_key_value(line) else {
                    return line.to_string();
                };

                if key.eq_ignore_ascii_case(UPDATE_BLOCK_KEY) {
                    found = true;
                    directive.clone()
                } else {
                    line.to_string()
                }
            })
            .collect();

        if !found {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(directive);
        }

        let mut output = lines.join("\n");
        output.push('\n');
        output
    }

    fn render_directive(&self) -> String {
        format!("{}={}\n", UPDATE_BLOCK_KEY, BLOCKED_VALUE)
    }

    fn parse_key_value(line: &str) -> Option<(&str, &str)> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            return None;
        }

        let (key, value) = trimmed.split_once('=')?;
        Some((key.trim(), value.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Steam root that passes the strict directory validation: the guard
    /// refuses to write into a folder without `steam.exe`, so the fake root
    /// must contain one.
    fn steam_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("steam.exe"), b"").expect("fake steam.exe");
        dir
    }

    fn guard(dir: &tempfile::TempDir) -> SteamUpdateGuard {
        SteamUpdateGuard::new(dir.path())
    }

    #[test]
    fn unblocking_deletes_steam_cfg_and_its_foreign_keys() {
        let dir = steam_dir();
        let cfg = dir.path().join("steam.cfg");
        fs::write(
            &cfg,
            "BootStrapperInhibitAll=Enable\nBootStrapperForceSelfUpdate=Disable\n",
        )
        .expect("seed steam.cfg");

        guard(&dir).unblock_updates().expect("unblock");

        // Re-enabling updates must leave Steam with no bootstrapper config at
        // all: an absent file cannot be malformed, while a rewritten one can.
        assert!(!cfg.exists(), "unblock must delete steam.cfg");
    }

    #[test]
    fn unblocking_without_a_file_is_a_noop() {
        let dir = steam_dir();

        guard(&dir).unblock_updates().expect("unblock");

        assert!(!dir.path().join("steam.cfg").exists());
    }

    #[test]
    fn blocking_writes_the_directive_and_reports_blocked() {
        let dir = steam_dir();
        let guard = guard(&dir);

        guard.block_updates().expect("block");

        let content = fs::read_to_string(dir.path().join("steam.cfg")).expect("read steam.cfg");
        assert!(content.contains("BootStrapperInhibitAll=Enable"));
        assert!(guard.is_blocked().expect("read state"));
    }

    #[test]
    fn blocking_replaces_the_directive_and_keeps_foreign_lines() {
        let dir = steam_dir();
        let cfg = dir.path().join("steam.cfg");
        fs::write(
            &cfg,
            "# my comment\nOtherKey=1\nbootstrapperinhibitall=Enable\n",
        )
        .expect("seed steam.cfg");

        guard(&dir).block_updates().expect("block");

        let content = fs::read_to_string(&cfg).expect("read steam.cfg");
        assert!(content.contains("# my comment"));
        assert!(content.contains("OtherKey=1"));
        // The key is matched case-insensitively and normalised, so a stale
        // spelling can never survive beside the directive Aether writes.
        assert_eq!(content.matches("=Enable").count(), 1);
        assert!(!content.contains("bootstrapperinhibitall=Enable"));
    }

    #[test]
    fn blocking_leaves_no_temporary_file_behind() {
        let dir = steam_dir();

        guard(&dir).block_updates().expect("block");

        let mut names: Vec<String> = fs::read_dir(dir.path())
            .expect("read dir")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["steam.cfg", "steam.exe"]);
    }

    #[test]
    fn is_blocked_ignores_a_commented_directive() {
        let dir = steam_dir();
        fs::write(
            dir.path().join("steam.cfg"),
            "#BootStrapperInhibitAll=Enable\n",
        )
        .expect("seed steam.cfg");

        assert!(!guard(&dir).is_blocked().expect("read state"));
    }

    #[test]
    fn a_directory_without_steam_exe_is_rejected() {
        let dir = tempfile::tempdir().expect("temp dir");

        let error = guard(&dir).unblock_updates().expect_err("must refuse");

        assert!(error.contains("steam.exe"), "unexpected error: {error}");
    }
}
