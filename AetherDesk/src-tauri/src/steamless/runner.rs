use crate::steamless::executable::{
    plausible_pe_machine, remove_stale_unpacked_outputs, unique_backup_path,
    unpacked_output_candidates, validate_executable,
};
use crate::steamless::tool_locator::SteamlessTool;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
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

    pub fn run(&self, request: &SteamlessRunRequest) -> Result<SteamlessRunResult, String> {
        validate_executable(&request.exe_path, &request.game_root)?;
        let expected_machine = {
            let mut input = fs::File::open(&request.exe_path).map_err(|e| e.to_string())?;
            plausible_pe_machine(&mut input)?
        };
        remove_stale_unpacked_outputs(&request.exe_path)?;

        let output = self.run_process(request)?;
        if output.timed_out || !output.status.success() {
            let detail =
                map_steamless_failure(request, &output.stdout, &output.stderr, output.timed_out);
            return Ok(failure_result(
                request,
                format!(
                    "{detail} Process status: {}. Original unchanged.",
                    output.status
                ),
                &output,
            ));
        }
        let Some(unpacked_path) = unpacked_output_candidates(&request.exe_path)
            .into_iter()
            .find(|candidate| candidate.is_file())
        else {
            return Ok(failure_result(
                request,
                map_steamless_failure(request, &output.stdout, &output.stderr, false),
                &output,
            ));
        };
        // Publish a stable, validated snapshot, never the tool's mutable pathname.
        let snapshot = match prepare_snapshot(&unpacked_path, expected_machine) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Ok(failure_result(
                    request,
                    format!("Steamless output rejected: {error}. Original unchanged."),
                    &output,
                ))
            }
        };
        if let Err(error) = fs::remove_file(&unpacked_path) {
            let _ = fs::remove_file(&snapshot);
            return Err(format!(
                "Could not retire validated Steamless output {}: {error}; original unchanged",
                unpacked_path.display()
            ));
        }
        let backup_path = unique_backup_path(&request.exe_path);
        let replacement = replace_executable(&request.exe_path, &snapshot, &backup_path);
        let _ = fs::remove_file(&snapshot);
        replacement?;
        let stdout_tail = tail(&output.stdout, 1500);
        let stderr_tail = tail(&output.stderr, 500);

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

        let mut stdout_pipe = child.stdout.take();
        let mut stderr_pipe = child.stderr.take();

        // Drain stdout and stderr in background reader threads to avoid OS pipe
        // buffer deadlock when Steamless outputs large unpacker diagnostics.
        let stdout_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut out) = stdout_pipe.take() {
                use std::io::Read;
                let _ = out.read_to_end(&mut buf);
            }
            buf
        });

        let stderr_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut err) = stderr_pipe.take() {
                use std::io::Read;
                let _ = err.read_to_end(&mut buf);
            }
            buf
        });

        let timeout = Duration::from_secs(request.timeout_seconds.max(1));
        let started = Instant::now();
        let mut timed_out = false;

        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() >= timeout => {
                    timed_out = true;
                    child.kill().map_err(|e| format!("Steamless timed out but could not be stopped (PID {}): {e}; original unchanged", child.id()))?;
                    break child
                        .wait()
                        .map_err(|e| format!("Wait for terminated Steamless: {e}"))?;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(format!("Failed while waiting for Steamless: {}", e)),
            }
        };

        let raw_stdout = stdout_reader.join().unwrap_or_default();
        let raw_stderr = stderr_reader.join().unwrap_or_default();

        Ok(SteamlessProcessOutput {
            stdout: String::from_utf8_lossy(&raw_stdout).to_string(),
            stderr: String::from_utf8_lossy(&raw_stderr).to_string(),
            timed_out,
            status,
        })
    }
}

struct SteamlessProcessOutput {
    stdout: String,
    stderr: String,
    timed_out: bool,
    status: ExitStatus,
}

fn failure_result(
    request: &SteamlessRunRequest,
    message: String,
    output: &SteamlessProcessOutput,
) -> SteamlessRunResult {
    SteamlessRunResult {
        success: false,
        cancelled: false,
        message,
        exe_path: Some(request.exe_path.display().to_string()),
        backup_path: None,
        stdout_tail: tail(&output.stdout, 1500),
        stderr_tail: tail(&output.stderr, 500),
    }
}

fn prepare_snapshot(candidate: &Path, expected_machine: u16) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(candidate).map_err(|e| e.to_string())?;
    let linked = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        linked || metadata.file_attributes() & 0x400 != 0
    };
    if linked || !metadata.is_file() {
        return Err("Output must be a regular, unlinked file".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // FILE_SHARE_READ only: no writer/deleter during copy.
    }
    let mut input = options
        .open(candidate)
        .map_err(|e| format!("Output is not closed/stable: {e}"))?;
    let parent = candidate.parent().ok_or("Steamless output has no parent")?;
    let snapshot = parent.join(format!(
        ".aether-steamless-{:016x}.tmp",
        rand::random::<u64>()
    ));
    let mut staged = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&snapshot)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        std::io::copy(&mut input, &mut staged).map_err(|e| e.to_string())?;
        staged.sync_all().map_err(|e| e.to_string())?;
        if plausible_pe_machine(&mut staged)? != expected_machine {
            return Err("Output architecture differs from the original".into());
        }
        staged
            .set_permissions(input.metadata().map_err(|e| e.to_string())?.permissions())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    drop(staged);
    if let Err(error) = result {
        let _ = fs::remove_file(&snapshot);
        return Err(error);
    }
    Ok(snapshot)
}

fn replace_executable(original: &Path, snapshot: &Path, backup: &Path) -> Result<(), String> {
    match fs::symlink_metadata(backup) {
        Ok(_) => {
            return Err(format!(
                "Refusing to overwrite existing Steamless backup {}",
                backup.display()
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    fs::rename(original, backup).map_err(|e| {
        format!(
            "Could not back up {}; original unchanged: {e}",
            original.display()
        )
    })?;
    if let Err(error) = fs::rename(snapshot, original) {
        return Err(restore_after_failed_replace(original, backup, &error));
    }
    Ok(())
}

fn restore_after_failed_replace(
    original: &Path,
    backup: &Path,
    replace_error: &std::io::Error,
) -> String {
    let restoration = match fs::symlink_metadata(original) {
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "live path is occupied; no overwrite during recovery",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::rename(backup, original),
        Err(error) => Err(error),
    };
    match restoration {
        Ok(()) => format!(
            "Could not replace executable: {replace_error}. Original restored at {}.",
            original.display()
        ),
        Err(restore_error) => {
            let recovery = if backup.is_file() {
                "Original backup retained at"
            } else {
                "Backup unavailable; last recovery location"
            };
            format!("Could not replace executable: {replace_error}. ORIGINAL NOT RESTORED: {restore_error}. {recovery} {} for manual recovery.", backup.display())
        }
    }
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
        return format!(
            "Steamless did not find the expected DRM protection in {}.",
            exe_name
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pe(machine: u16, marker: u8) -> Vec<u8> {
        let mut bytes = vec![0_u8; 1024];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&128_u32.to_le_bytes());
        bytes[128..132].copy_from_slice(b"PE\0\0");
        bytes[132..134].copy_from_slice(&machine.to_le_bytes());
        bytes[134..136].copy_from_slice(&1_u16.to_le_bytes());
        let (optional_size, magic, base) = if machine == 0x8664 {
            (240_u16, 0x20b_u16, 112_usize)
        } else {
            (224_u16, 0x10b_u16, 96_usize)
        };
        bytes[148..150].copy_from_slice(&optional_size.to_le_bytes());
        bytes[150..152].copy_from_slice(&2_u16.to_le_bytes());
        let optional = 152;
        bytes[optional..optional + 2].copy_from_slice(&magic.to_le_bytes());
        for (offset, value) in [
            (16, 4096_u32),
            (32, 4096),
            (36, 512),
            (56, 8192),
            (60, 512),
            (base - 4, 16),
        ] {
            bytes[optional + offset..optional + offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let section = optional + usize::from(optional_size);
        bytes[section..section + 5].copy_from_slice(b".text");
        for (offset, value) in [(8, 512_u32), (12, 4096), (16, 512), (20, 512)] {
            bytes[section + offset..section + offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[512..].fill(marker);
        bytes
    }

    fn fixture(
        output: &[u8],
        exit_code: i32,
        hang: bool,
    ) -> (tempfile::TempDir, SteamlessRunner, SteamlessRunRequest) {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("game");
        let tool = temp.path().join("fake-tool");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&tool).unwrap();
        let exe_path = game.join("Game.exe");
        fs::write(&exe_path, pe(0x8664, 0xcc)).unwrap();
        fs::write(temp.path().join("protected-sentinel.txt"), b"protected").unwrap();
        fs::write(tool.join("payload.bin"), output).unwrap();
        #[cfg(windows)]
        let (name, script) = ("fake.cmd", format!("@echo off\r\necho started>\"%~dp0started.txt\"\r\ncopy /b /y \"%~dp0payload.bin\" \"%~4.unpacked.exe\" >nul\r\n{}", if hang { ":hang\r\ngoto hang\r\n".to_string() } else { format!("exit /B {exit_code}\r\n") }));
        #[cfg(unix)]
        let (name, script) = ("fake.sh", format!("#!/bin/sh\nprintf started >\"$PWD/started.txt\"\ncp \"$PWD/payload.bin\" \"$4.unpacked.exe\"\n{}", if hang { "while :; do :; done\n".to_string() } else { format!("exit {exit_code}\n") }));
        let cli = tool.join(name);
        fs::write(&cli, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let runner = SteamlessRunner::new(SteamlessTool {
            cli_path: cli,
            working_dir: tool,
        });
        let request = SteamlessRunRequest {
            exe_path,
            game_root: game,
            // Only the hang test exercises the 1s enforcement. Success-path
            // tests get headroom: on Windows a cold process spawn (Defender /
            // AMSI scan of a fresh script in a fresh temp dir, amplified by
            // parallel test threads) can exceed 1s, which used to surface as
            // a generic "timed out" failure instead of the real outcome.
            timeout_seconds: if hang { 1 } else { 30 },
        };
        (temp, runner, request)
    }

    #[test]
    fn nonzero_exit_rejects_even_a_valid_candidate_without_touching_original() {
        let (temp, runner, request) = fixture(&pe(0x8664, 0x90), 7, false);
        let result = runner.run(&request).unwrap();
        assert!(!result.success);
        assert!(result.backup_path.is_none());
        assert_eq!(fs::read(&request.exe_path).unwrap(), pe(0x8664, 0xcc));
        assert_eq!(
            fs::read(temp.path().join("protected-sentinel.txt")).unwrap(),
            b"protected"
        );
    }

    #[test]
    fn timeout_after_partial_output_never_replaces_original() {
        let (_temp, runner, request) = fixture(b"MZpartial", 0, true);
        let result = runner.run(&request).unwrap();
        assert!(!result.success);
        assert!(result.message.contains("timed out"));
        assert_eq!(fs::read(&request.exe_path).unwrap(), pe(0x8664, 0xcc));
        assert!(result.backup_path.is_none());
    }

    #[test]
    fn successful_process_with_empty_or_truncated_pe_is_not_successful_unpacking() {
        for output in [Vec::new(), b"MZ".to_vec(), pe(0x8664, 0x90)[..512].to_vec()] {
            let (_temp, runner, request) = fixture(&output, 0, false);
            let result = runner.run(&request).unwrap();
            assert!(!result.success);
            assert_eq!(fs::read(&request.exe_path).unwrap(), pe(0x8664, 0xcc));
            assert!(result.backup_path.is_none());
        }
    }

    #[test]
    fn wrong_architecture_is_rejected_before_creating_backup() {
        let (_temp, runner, request) = fixture(&pe(0x014c, 0x90), 0, false);
        let result = runner.run(&request).unwrap();
        assert!(!result.success);
        assert!(
            !result.message.contains("timed out"),
            "slow spawn masked the real outcome: {}",
            result.message
        );
        assert!(result.message.contains("architecture"));
        assert_eq!(fs::read(&request.exe_path).unwrap(), pe(0x8664, 0xcc));
        assert!(result.backup_path.is_none());
    }

    #[test]
    fn valid_successful_output_replaces_live_file_and_preserves_exact_original() {
        let (_temp, runner, request) = fixture(&pe(0x8664, 0x90), 0, false);
        let result = runner.run(&request).unwrap();
        assert!(
            !result.message.contains("timed out"),
            "slow spawn masked the real outcome: {}",
            result.message
        );
        assert!(result.success);
        assert!(unpacked_output_candidates(&request.exe_path)
            .iter()
            .all(|path| !path.exists()));
        assert_eq!(fs::read(&request.exe_path).unwrap(), pe(0x8664, 0x90));
        assert_eq!(
            fs::read(result.backup_path.unwrap()).unwrap(),
            pe(0x8664, 0xcc)
        );
        assert!(fs::read_dir(&request.game_root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".aether-steamless-")));
    }

    #[test]
    fn failed_stale_cleanup_stops_before_launching_the_tool() {
        let (temp, runner, request) = fixture(&pe(0x8664, 0x90), 0, false);
        fs::create_dir(request.exe_path.with_file_name("Game.exe.unpacked.exe")).unwrap();
        assert!(runner.run(&request).is_err());
        assert!(!temp.path().join("fake-tool/started.txt").exists());
        assert_eq!(fs::read(&request.exe_path).unwrap(), pe(0x8664, 0xcc));
    }

    #[test]
    fn failed_publication_reports_an_actually_restored_original() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("game.exe");
        let backup = temp.path().join("original.bak");
        fs::write(&original, b"original sentinel").unwrap();
        let error =
            replace_executable(&original, &temp.path().join("missing.tmp"), &backup).unwrap_err();
        assert!(error.contains("Original restored"));
        assert_eq!(fs::read(original).unwrap(), b"original sentinel");
        assert!(!backup.exists());
    }

    #[test]
    fn failed_restore_reports_failure_and_retains_backup_and_collision_sentinel() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("game.exe");
        let backup = temp.path().join("original.bak");
        fs::write(&original, b"original sentinel").unwrap();
        fs::rename(&original, &backup).unwrap();
        fs::create_dir(&original).unwrap();
        fs::write(original.join("sentinel.txt"), b"collision sentinel").unwrap();
        let replace_error = fs::rename(temp.path().join("missing.tmp"), &original).unwrap_err();
        let error = restore_after_failed_replace(&original, &backup, &replace_error);
        assert!(error.contains("ORIGINAL NOT RESTORED"));
        assert!(error.contains(&backup.display().to_string()));
        assert_eq!(fs::read(backup).unwrap(), b"original sentinel");
        assert_eq!(
            fs::read(original.join("sentinel.txt")).unwrap(),
            b"collision sentinel"
        );
    }

    #[test]
    fn existing_backup_is_never_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("game.exe");
        let backup = temp.path().join("prior.bak");
        let snapshot = temp.path().join("new.tmp");
        fs::write(&original, b"original").unwrap();
        fs::write(&backup, b"old backup sentinel").unwrap();
        fs::write(&snapshot, b"new").unwrap();
        assert!(replace_executable(&original, &snapshot, &backup).is_err());
        assert_eq!(fs::read(backup).unwrap(), b"old backup sentinel");
        assert_eq!(fs::read(original).unwrap(), b"original");
    }

    #[test]
    fn invalid_section_ranges_or_optional_header_never_pass_snapshot_validation() {
        let temp = tempfile::tempdir().unwrap();
        let candidate = temp.path().join("candidate.exe");
        for offset in [152, 152 + 60, 152 + 240 + 20] {
            let mut output = pe(0x8664, 0x90);
            output[offset..offset + 4].fill(0xff);
            fs::write(&candidate, output).unwrap();
            assert!(prepare_snapshot(&candidate, 0x8664).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn output_with_an_open_writer_is_not_accepted_as_stable() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let candidate = temp.path().join("candidate.exe");
        fs::write(&candidate, pe(0x8664, 0x90)).unwrap();
        let writer = fs::OpenOptions::new()
            .write(true)
            .share_mode(7)
            .open(&candidate)
            .unwrap();
        assert!(prepare_snapshot(&candidate, 0x8664).is_err());
        drop(writer);
        let snapshot = prepare_snapshot(&candidate, 0x8664).unwrap();
        fs::remove_file(snapshot).unwrap();
    }

    #[test]
    fn restore_never_overwrites_a_new_regular_file_collision() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("game.exe");
        let backup = temp.path().join("backup.bak");
        fs::write(&backup, b"original backup").unwrap();
        fs::write(&original, b"new user sentinel").unwrap();
        let replacement_error = fs::rename(temp.path().join("missing.tmp"), &original).unwrap_err();
        let message = restore_after_failed_replace(&original, &backup, &replacement_error);
        assert!(message.contains("ORIGINAL NOT RESTORED"));
        assert_eq!(fs::read(original).unwrap(), b"new user sentinel");
        assert_eq!(fs::read(backup).unwrap(), b"original backup");
    }

    #[cfg(windows)]
    #[test]
    fn restore_rename_failure_from_a_locked_backup_is_reported_with_recovery_path() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("game.exe");
        let backup = temp.path().join("backup.bak");
        fs::write(&backup, b"original backup").unwrap();
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&backup)
            .unwrap();
        let replacement_error = fs::rename(temp.path().join("missing.tmp"), &original).unwrap_err();
        let message = restore_after_failed_replace(&original, &backup, &replacement_error);
        assert!(message.contains("ORIGINAL NOT RESTORED"));
        assert!(message.contains(&backup.display().to_string()));
        assert!(backup.is_file());
        assert!(!original.exists());
        drop(locked);
        fs::rename(&backup, &original).unwrap();
        assert_eq!(fs::read(original).unwrap(), b"original backup");
    }
}
