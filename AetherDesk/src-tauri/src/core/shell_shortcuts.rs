//! Windows Start Menu / Desktop shortcuts and Explorer icon-cache refresh.
//!
//! The title-bar icon is set with `Window::set_icon`. Desktop, Start Menu and
//! Explorer folders show the icon of the `.lnk` (or of the `.exe` resource).
//! We keep those shortcuts pointed at a stable `config/icons/shell.ico` and
//! ask the shell to reload so Windows' icon cache updates without restarting Explorer.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

const SHELL_ICO_NAME: &str = "shell.ico";

pub fn shell_ico_path() -> PathBuf {
    crate::core::custom_css::icons_dir().join(SHELL_ICO_NAME)
}

/// Write the ICO used by shortcuts. Overwrites in place; the shell is then
/// notified so the cache picks up the new bytes.
pub fn materialize_shell_ico(source: Option<&Path>) -> Result<PathBuf, String> {
    let dest = shell_ico_path();
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create icons folder: {}", e))?;
    }

    match source {
        Some(src) if is_ico(src) => {
            fs::copy(src, &dest)
                .map_err(|e| format!("Failed to copy icon to {}: {}", dest.display(), e))?;
        }
        Some(src) => {
            let bytes = fs::read(src)
                .map_err(|e| format!("Failed to read {}: {}", src.display(), e))?;
            write_decoded_ico(&bytes, &dest)?;
        }
        None => {
            write_decoded_ico(crate::core::custom_css::DEFAULT_AETHER_ICON, &dest)
                .or_else(|_| {
                    fs::write(&dest, crate::core::custom_css::DEFAULT_AETHER_ICON)
                        .map_err(|e| format!("Failed to write default shell.ico: {}", e))
                })?;
        }
    }

    Ok(dest)
}

fn is_ico(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("ico"))
        .unwrap_or(false)
}

fn write_decoded_ico(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let decoded = image::load_from_memory(bytes)
        .map_err(|e| format!("Unsupported icon format: {}", e))?;
    let mut out = Vec::new();
    decoded
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Ico)
        .map_err(|e| format!("Failed to encode ICO: {}", e))?;
    fs::write(dest, out).map_err(|e| format!("Failed to write {}: {}", dest.display(), e))
}

/// Create/update Start Menu + existing Desktop shortcuts and refresh Explorer.
pub fn sync_windows_shortcuts(icon_ico: &Path) {
    #[cfg(target_os = "windows")]
    windows_impl::sync(icon_ico);
    #[cfg(not(target_os = "windows"))]
    let _ = icon_ico;
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn sync(icon_ico: &Path) {
        let Ok(exe_path) = std::env::current_exe() else { return };
        if exe_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.eq_ignore_ascii_case("aether_updater.exe"))
            .unwrap_or(false)
        {
            return;
        }
        let Some(work_dir) = exe_path.parent() else { return };

        let mut targets = Vec::new();
        if let Ok(appdata) = std::env::var("APPDATA") {
            let programs = Path::new(&appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs");
            let _ = fs::create_dir_all(&programs);
            targets.push(programs.join("AetherDesk.lnk"));
        }
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            let desktop = Path::new(&userprofile).join("Desktop").join("AetherDesk.lnk");
            if desktop.exists() {
                targets.push(desktop);
            }
        }
        if let Ok(public) = std::env::var("PUBLIC") {
            let desktop = Path::new(&public).join("Desktop").join("AetherDesk.lnk");
            if desktop.exists() {
                targets.push(desktop);
            }
        }

        let exe = ps_quote(&exe_path);
        let work = ps_quote(work_dir);
        let icon = ps_quote(icon_ico);

        for shortcut in targets {
            let create_if_missing = shortcut
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.eq_ignore_ascii_case("AetherDesk.lnk"))
                .unwrap_or(false)
                && shortcut
                    .parent()
                    .map(|p| p.ends_with("Programs"))
                    .unwrap_or(false);

            if !shortcut.exists() && !create_if_missing {
                continue;
            }

            let lnk = ps_quote(&shortcut);
            let script = format!(
                "$ws = New-Object -ComObject WScript.Shell; $s = $ws.CreateShortcut({lnk}); $s.TargetPath = {exe}; $s.WorkingDirectory = {work}; $s.Description = 'AetherDesk - Steam Library Manager'; $s.IconLocation = {icon} + ',0'; $s.Save();",
                lnk = lnk,
                exe = exe,
                work = work,
                icon = icon,
            );
            let _ = Command::new("powershell.exe")
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }

        refresh_icon_cache();
    }

    fn ps_quote(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "''"))
    }

    fn refresh_icon_cache() {
        // Lightweight: tells Explorer to rebuild per-user icons without killing it.
        let _ = Command::new("ie4uinit.exe")
            .arg("-show")
            .creation_flags(CREATE_NO_WINDOW)
            .status();

        unsafe {
            windows_sys::Win32::UI::Shell::SHChangeNotify(
                // windows-sys 0.59: SHCNE_* is u32, SHChangeNotify.wEventId is i32
                windows_sys::Win32::UI::Shell::SHCNE_ASSOCCHANGED as i32,
                windows_sys::Win32::UI::Shell::SHCNF_IDLIST,
                std::ptr::null(),
                std::ptr::null(),
            );
        }
    }
}
