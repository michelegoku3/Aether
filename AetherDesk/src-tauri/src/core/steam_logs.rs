//! Collecting the selected Steam client log files from `<Steam>\logs` for the
//! exported bundle, capped to the last 12 hours of their own stamps. Steam
//! writes every selected log with the same line shape, so one filter covers
//! all of them. Nothing here touches the Aether log line contract
//! (`docs/shared_contracts.md` §10): these are Steam's own documents, kept as
//! Steam wrote them.

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
pub fn cutoff_key_12h_before_now() -> String {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Time::{
        FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime,
    };

    let window_ticks = (STEAM_WINDOW_HOURS as i64) * 3_600 * 10_000_000;
    let mut raw = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    // SAFETY: FILETIME is a plain Windows FFI struct and ours.
    unsafe {
        GetSystemTimeAsFileTime(&mut raw);
    }
    let cutoff = ((raw.dwHighDateTime as i64) << 32 | raw.dwLowDateTime as i64) - window_ticks;
    let as_file_time = FILETIME {
        dwLowDateTime: cutoff as u32,
        dwHighDateTime: (cutoff >> 32) as u32,
    };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: both outputs are ours; the null zone argument asks for the
    // system's own dynamic time zone.
    unsafe {
        FileTimeToSystemTime(&as_file_time, &mut utc);
        SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local);
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
    let cut = content.len() - fallback_max as usize;
    let start = match content[cut..].find('\n') {
        Some(pos) => cut + pos + 1,
        // A single line longer than the cap: hand over exactly the cap's
        // worth of tail, mid-line. Nothing useful is lost at 4 MiB scale.
        None => cut,
    };
    Some(content[start..].as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::filter_steam_log_content;

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
}
