use crate::steamless::executable::{
    remove_stale_unpacked_outputs, unique_backup_path, unpacked_output_candidates,
    validate_executable,
};
use crate::steamless::tool_locator::SteamlessTool;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SteamlessRunRequest {
    pub exe_path: PathBuf,
    pub game_root: PathBuf,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamlessRunResult {
    pub success: bool,
    pub cancelled: bool,
    pub message: String,
    pub exe_path: Option<String>,
    pub backup_path: Option<String>,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

pub struct SteamlessRunner {
    tool: SteamlessTool,
}

impl SteamlessRunner {
    pub fn new(tool: SteamlessTool) -> Self {
        Self { tool }
    }

    pub fn run(&self, request: SteamlessRunRequest) -> Result<SteamlessRunResult, String> {
        validate_executable(&request.exe_path, &request.game_root)?;
        remove_stale_unpacked_outputs(&request.exe_path);

        let output = self.run_process(&request)?;
        let stdout_tail = tail(&output.stdout, 1500);
        let stderr_tail = tail(&output.stderr, 500);

        let Some(unpacked_path) = unpacked_output_candidates(&request.exe_path)
            .into_iter()
            .find(|candidate| candidate.is_file())
        else {
            return Ok(SteamlessRunResult {
                success: false,
                cancelled: false,
                message: map_steamless_failure(
                    &request,
                    &output.stdout,
                    &output.stderr,
                    output.timed_out,
                ),
                exe_path: Some(request.exe_path.display().to_string()),
                backup_path: None,
                stdout_tail,
                stderr_tail,
            });
        };

        let backup_path = unique_backup_path(&request.exe_path);
        fs::rename(&request.exe_path, &backup_path).map_err(|e| {
            format!(
                "Steamless unpacked {}, but AetherDesk could not back up the original executable: {}. Close the game/launcher and try again.",
                request.exe_path.file_name().and_then(|name| name.to_str()).unwrap_or("the executable"),
                e
            )
        })?;

        if let Err(replace_error) = fs::rename(&unpacked_path, &request.exe_path) {
            let _ = fs::rename(&backup_path, &request.exe_path);
            return Err(format!(
                "Steamless unpacked the executable but AetherDesk could not replace the original: {}. The original backup was restored.",
                replace_error
            ));
        }

        let exe_name = request
            .exe_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Executable");
        let backup_name = backup_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("backup");

        Ok(SteamlessRunResult {
            success: true,
            cancelled: false,
            message: format!(
                "Steamless unpacked {} successfully. Original saved as {}.",
                exe_name, backup_name
            ),
            exe_path: Some(request.exe_path.display().to_string()),
            backup_path: Some(backup_path.display().to_string()),
            stdout_tail,
            stderr_tail,
        })
    }

    fn run_process(&self, request: &SteamlessRunRequest) -> Result<SteamlessProcessOutput, String> {
        let mut command = Command::new(&self.tool.cli_path);
        command
            .arg("--exp")
            .arg("--realign")
            .arg("--recalcchecksum")
            .arg(&request.exe_path)
            .current_dir(&self.tool.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }

        let mut child = command.spawn().map_err(|e| {
            format!(
                "Failed to launch Steamless at {}: {}",
                self.tool.cli_path.display(),
                e
            )
        })?;

        let timeout = Duration::from_secs(request.timeout_seconds.max(1));
        let started = Instant::now();
        let mut timed_out = false;

        loop {
            match child.try_wait() {
                Ok(Some(_status)) => break,
                Ok(None) if started.elapsed() >= timeout => {
                    timed_out = true;
                    let _ = child.kill();
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => return Err(format!("Failed while waiting for Steamless: {}", e)),
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to collect Steamless output: {}", e))?;

        Ok(SteamlessProcessOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            timed_out,
        })
    }
}

struct SteamlessProcessOutput {
    stdout: String,
    stderr: String,
    timed_out: bool,
}

fn map_steamless_failure(
    request: &SteamlessRunRequest,
    stdout: &str,
    stderr: &str,
    timed_out: bool,
) -> String {
    let exe_name = request
        .exe_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("the selected executable");

    if timed_out {
        return format!("Steamless timed out while processing {}.", exe_name);
    }

    let combined = format!("{}\n{}", stdout, stderr).to_lowercase();
    if combined.contains("invalid input file") {
        return format!(
            "Steamless rejected {} as an invalid input file. Pick the main game executable and try again.",
            exe_name
        );
    }

    if combined.contains("all unpackers failed") {
        return format!(
            "Steamless detected protected-code markers in {}, but none of the bundled unpackers support this wrapper variant yet.",
            exe_name
        );
    }

    if combined.contains("not packed")
        || combined.contains("is not packed")
        || combined.contains("no .bind section")
    {
        return format!("Steamless did not find the expected DRM protection in {}.", exe_name);
    }

    format!(
        "Steamless did not produce an unpacked executable for {}. The file may not be protected, or may use an unsupported wrapper variant.",
        exe_name
    )
}

fn tail(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    let start = chars.len().saturating_sub(max_chars);
    chars[start..].iter().collect::<String>().trim().to_string()
}
