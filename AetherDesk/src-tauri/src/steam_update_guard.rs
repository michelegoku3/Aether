use std::fs;
use std::path::PathBuf;

const STEAM_CFG_FILE_NAME: &str = "steam.cfg";
const UPDATE_BLOCK_KEY: &str = "BootStrapperInhibitAll";
const BLOCKED_VALUE: &str = "Enable";
const UNBLOCKED_VALUE: &str = "Disable";

/// Owns every read/write operation related to Steam's update guard config.
///
/// The UI and Tauri commands should not know how `steam.cfg` is structured:
/// they only ask whether updates are blocked or request a state change.
/// This keeps file parsing/writing isolated and easier to maintain.
pub struct SteamUpdateGuard {
    steam_dir: PathBuf,
}

impl SteamUpdateGuard {
    pub fn new(steam_path: impl Into<PathBuf>) -> Self {
        Self {
            steam_dir: steam_path.into(),
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

            key.eq_ignore_ascii_case(UPDATE_BLOCK_KEY)
                && value.eq_ignore_ascii_case(BLOCKED_VALUE)
        }))
    }

    pub fn block_updates(&self) -> Result<(), String> {
        self.set_blocked(true)
    }

    pub fn unblock_updates(&self) -> Result<(), String> {
        self.set_blocked(false)
    }

    fn set_blocked(&self, blocked: bool) -> Result<(), String> {
        self.validate_steam_dir()?;

        let cfg_path = self.config_path();
        if !cfg_path.exists() && !blocked {
            return Ok(());
        }

        let content = if cfg_path.exists() {
            self.read_config()?
        } else {
            String::new()
        };
        let next_content = self.upsert_update_directive(&content, blocked);

        fs::write(cfg_path, next_content)
            .map_err(|e| format!("Failed to update steam.cfg: {}", e))
    }

    fn validate_steam_dir(&self) -> Result<(), String> {
        if !self.steam_dir.exists() {
            return Err(format!(
                "Steam installation path does not exist: {}",
                self.steam_dir.display()
            ));
        }

        if !self.steam_dir.is_dir() {
            return Err(format!(
                "Steam installation path is not a directory: {}",
                self.steam_dir.display()
            ));
        }

        Ok(())
    }

    fn read_config(&self) -> Result<String, String> {
        fs::read_to_string(self.config_path())
            .map_err(|e| format!("Failed to read steam.cfg: {}", e))
    }

    fn upsert_update_directive(&self, content: &str, blocked: bool) -> String {
        let directive = self.render_directive(blocked).trim_end().to_string();
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

    fn render_directive(&self, blocked: bool) -> String {
        let value = if blocked { BLOCKED_VALUE } else { UNBLOCKED_VALUE };
        format!("{}={}\n", UPDATE_BLOCK_KEY, value)
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
