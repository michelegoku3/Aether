//! Collecting the selected Steam client log files from `<Steam>\logs` for the
//! exported bundle, capped to the last 12 hours of their own stamps. Steam
//! writes every selected log with the same line shape, so one filter covers
//! all of them. Nothing here touches the Aether log line contract
//! (`docs/shared_contracts.md` §10): these are Steam's own documents, kept as
//! Steam wrote them.

use std::path::Path;
use std::time::SystemTime;

/// How far back the export keeps Steam log lines.
pub const STEAM_WINDOW_HOURS: i64 = 12;

/// Cap for a collected file whose lines carry no Steam stamp at all
/// (Steam's `console_log`-style free-form output).
pub const STEAM_FALLBACK_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Local wall-clock stamp (`YYYY-MM-DD HH:MM:SS`) for now minus the window.
/// The moment is cut on the UTC FILETIME timeline (no DST drift while
/// subtracting) and only then rendered as local time, so a window that spans
/// a DST shift skews the cutoff by at most the difference between the two
/// offsets it crosses.
#[cfg(target_os = "windows")]
pub fn cutoff_key_12h_before_now() -> String {    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Time::{
        FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime,
    };

    let window_ticks = STEAM_WINDOW_HOURS * 3_600 * 10_000_000;
    let mut raw = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    // SAFETY: FILETIME is a plain Windows FFI struct and ours.
    unsafe {
        GetSystemTimeAsFileTime(&mut raw);
    }
    let cutoff = ((raw.dwHighDateTime as i64) << 32 | raw.dwLowDateTime as i64) - window_ticks;
    // FILETIME is the low/high split of a 64-bit tick count; the `as` casts
    // below re-assemble exactly the value built above, nothing is truncated.
    let as_file_time = FILETIME {
        dwLowDateTime: (cutoff & 0xFFFF_FFFF) as u32,
        dwHighDateTime: ((cutoff >> 32) & 0xFFFF_FFFF) as u32,
    };
    // SAFETY: SYSTEMTIME is a plain Windows FFI struct, so zero
    // initialization is valid before each call fills it.
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: same plain FFI struct, ours alone until the conversion.
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: both outputs are ours; the null zone argument asks for the
    // system's own dynamic time zone.
    unsafe {
        let _ = FileTimeToSystemTime(&as_file_time, &mut utc);
        let _ = SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local);
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        local.wYear,
        local.wMonth,
        local.wDay,
        local.wHour,
        local.wMinute,
        local.wSecond
    )
}

#[cfg(not(target_os = "windows"))]
pub fn cutoff_key_12h_before_now() -> String {
    // Non-Windows Desk builds are not a shipping target; with this stub the
    // stamp filter keeps every line of the files it is given.
    "0000-01-01 00:00:00".to_string()
}

/// The Steam client logs the bundle asks for (each also always tried as a
/// `.previous.txt` twin, Steam's own session rotation). Chosen by diagnosis
/// value: installs and download failures (`content_log`, `console_log`,
/// `bootstrap_log`, `installscript_log`), connectivity (`connection_log`),
/// game crashes (`gameprocess_log`) and Workshop manifests (`workshop_log`).
pub const SELECTED_STEAM_LOGS: &[&str] = &[
    "bootstrap_log.txt",
    "console_log.txt",
    "connection_log.txt",
    "content_log.txt",
    "gameprocess_log.txt",
    "installscript_log.txt",
    "workshop_log.txt",
];

/// Collects the selected Steam client logs from `<steam_root>\logs`, capped
/// to the window (stale mtime or stale stamps drop a file). Returns
/// `(file_name, filtered_bytes)`, names as Steam wrote them, in
/// `SELECTED_STEAM_LOGS` order. Read errors on one file never abort the
/// collection: that file is just absent from the bundle.
pub fn collect_steam_logs(
    steam_root: &Path,
    cutoff_instant: SystemTime,
    cutoff_stamp: &str,
) -> Vec<(String, Vec<u8>)> {
    let logs_dir = steam_root.join("logs");
    let mut collected = Vec::new();
    for name in SELECTED_STEAM_LOGS {
        let base = name.trim_end_matches(".txt");
        for candidate in [(*name).to_string(), format!("{base}.previous.txt")] {
            let fresh = std::fs::metadata(logs_dir.join(&candidate))
                .ok()
                .filter(|meta| meta.is_file())
                .and_then(|meta| meta.modified().ok())
                .is_some_and(|modified| modified >= cutoff_instant);
            if !fresh {
                continue;
            }
            let Ok(raw) = std::fs::read(logs_dir.join(&candidate)) else {
                continue;
            };
            let content = String::from_utf8_lossy(&raw).into_owned();
            let Some(bytes) =
                filter_steam_log_content(&content, cutoff_stamp, STEAM_FALLBACK_MAX_BYTES)
            else {
                continue;
            };
            collected.push((candidate, bytes));
        }
    }
    collected
}

/// Steam's own line head: `[YYYY-MM-DD HH:MM:SS] message`. Returns the stamp
/// so the caller can order it against another local stamp.
fn parse_steam_stamp(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('[')?;
    let stamp = inner.get(..19)?;
    let bytes = stamp.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        let ok = match i {
            4 | 7 => *byte == b'-',
            10 => *byte == b' ',
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        };
        if !ok {
            return None;
        }
    }
    if inner.as_bytes().get(19) != Some(&b']') {
        return None;
    }
    Some(stamp)
}

/// Keeps the lines stamped within the window. Files Steam writes without
/// stamps (see `STEAM_FALLBACK_MAX_BYTES`) are kept whole, tail-capped. A
/// file whose every stamped line falls before the window returns `None` so
/// the collector drops it instead of appending an empty document.
fn filter_steam_log_content(content: &str, cutoff: &str, fallback_max: u64) -> Option<Vec<u8>> {
    // A bracket-headed line is Steam's stamp contract: one that fails the
    // exact shape means Steam changed format and nobody should ship the
    // mangled document, so the file is rejected outright.
    let mut has_valid_stamp = false;
    for line in content.lines() {
        if line.starts_with('[') {
            match parse_steam_stamp(line) {
                Some(_) => has_valid_stamp = true,
                None => return None,
            }
        }
    }
    if has_valid_stamp {
        let kept: Vec<&str> = content
            .lines()
            .filter_map(|line| {
                let stamp = parse_steam_stamp(line)?;
                (stamp >= cutoff).then_some(line)
            })
            .collect();
        if kept.is_empty() {
            return None;
        }
        return Some(kept.join("\n").into_bytes());
    }

    // No stamps anywhere: the whole document is the artifact, cut only when
    // it would blow the bundle past the cap. The tail is aligned to a line
    // start so the first exported line is whole.
    if (content.len() as u64) <= fallback_max {
        return Some(content.as_bytes().to_vec());
    }
    // The cap is defined on files Steam writes (never near `usize::MAX`), so
    // a 32-bit-target overflow would only allow a bigger tail.
    let max = usize::try_from(fallback_max).unwrap_or(usize::MAX);
    let cut = content.len() - max;
    let start = match content[cut..].find('\n') {
        Some(pos) => cut + pos + 1,
        // A single line longer than the cap: hand over exactly the cap's
        // worth of tail, mid-line. Nothing useful is lost at 4 MiB scale.
        None => cut,
    };
    Some(content.as_bytes()[start..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::{collect_steam_logs, filter_steam_log_content};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    /// Fresh-cutoff for the fixture: anything written just now counts as
    /// in-window, so only tests that explicitly age files hit the gate.
    fn fresh_cutoff() -> SystemTime {
        SystemTime::now() - Duration::from_secs(2 * 24 * 3600)
    }

    fn write_fresh(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("fixture write");
        path
    }

    fn set_old_mtime(path: &Path, text: &str) {
        std::fs::write(path, text).expect("fixture write");
        let old = filetime::FileTime::from_unix_time(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
                - 2 * 24 * 3600,
            0,
        );
        filetime::set_file_times(path, old, old).expect("fixture mtime");
    }

    fn content_with_stamps() -> String {
        "[2026-10-30 05:59:59] old line\n[2026-10-30 12:00:00] cutoff line\n[2026-10-30 12:00:01] fresh line\n"
            .to_string()
    }

    #[test]
    fn stamp_lines_older_than_cutoff_are_dropped() {
        let kept = filter_steam_log_content(
            &content_with_stamps(),
            "2026-10-30 12:00:00",
            u64::MAX,
        )
        .expect("file with fresh lines is collected");
        let kept = String::from_utf8_lossy(&kept);
        assert!(kept.contains("cutoff line") && kept.contains("fresh line"));
        assert!(!kept.contains("old line"));
    }

    #[test]
    fn every_line_old_returns_nothing() {
        assert!(
            filter_steam_log_content(
                &content_with_stamps(),
                "2027-01-01 00:00:00",
                u64::MAX
            )
            .is_none()
        );
    }

    #[test]
    fn no_stamp_file_is_included_whole() {
        let kept = filter_steam_log_content(
            "plain line\nno stamp here\n",
            "2026-10-30 12:00:00",
            u64::MAX,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&kept).as_ref(),
            "plain line\nno stamp here\n"
        );
    }

    #[test]
    fn no_stamp_file_is_tail_capped_on_a_line_boundary() {
        let body = "abcdefghij\n".repeat(10); // 110 bytes
        let kept = filter_steam_log_content(&body, "2026-10-30 12:00:00", 50)
            .unwrap();
        assert!(kept.len() <= 50);
        let kept = String::from_utf8_lossy(&kept);
        assert_eq!(kept.lines().count(), 4, "only whole lines are kept");
        assert!(kept.ends_with("abcdefghij\n"), "last line stays whole");
    }

    #[test]
    fn stamp_requires_the_bracketed_shape() {
        assert!(
            filter_steam_log_content("[garbage] hello\n", "2026-10-30 12:00:00", u64::MAX)
                .is_none()
        );
    }

    #[test]
    fn only_selected_names_and_previous_twins_within_window_are_collected() {
        let root = tempfile::tempdir().unwrap();
        let logs = root.path().join("logs");
        std::fs::create_dir(&logs).unwrap();
        write_fresh(&logs, "content_log.txt", &content_with_stamps());
        write_fresh(&logs, "console_log.txt", "plain no stamps\n");
        write_fresh(&logs, "controller.txt", "not selected\n");
        set_old_mtime(
            &logs.join("content_log.previous.txt"),
            "previous text\n",
        );
        set_old_mtime(&logs.join("bootstrap_log.txt"), &content_with_stamps());

        let collected =
            collect_steam_logs(root.path(), fresh_cutoff(), "2026-10-30 12:00:00");
        let names: Vec<&str> = collected.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["console_log.txt", "content_log.txt"]);
    }

    #[test]
    fn steam_root_without_logs_dir_returns_nothing() {
        let root = tempfile::tempdir().unwrap();
        assert!(collect_steam_logs(root.path(), fresh_cutoff(), "2026-10-30 12:00:00").is_empty());
    }

    #[test]
    fn stale_content_is_not_collected() {
        let root = tempfile::tempdir().unwrap();
        let logs = root.path().join("logs");
        std::fs::create_dir(&logs).unwrap();
        // Fresh mtime, but every line is older than the cutoff → excluded.
        write_fresh(&logs, "content_log.txt", &content_with_stamps());
        let collected = collect_steam_logs(root.path(), fresh_cutoff(), "2027-01-01 00:00:00");
        assert!(collected.is_empty());
    }
}
