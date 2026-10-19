//! Shared helpers for native dialog results.
//!
//! Every picker command (`pick_*`) converts Tauri [`FilePath`] values to plain
//! strings the same way; keeping the conversion here avoids a private copy in
//! each command module (DRY).

use tauri_plugin_dialog::FilePath;

/// Convert a picked [`FilePath`] into a path string.
///
/// Returns `Ok(None)` when the dialog result carries no usable path (treated as
/// "cancelled"). A picked path that is not valid Unicode is an explicit `Err`
/// instead of a silently corrupted (`U+FFFD`) path that would later fail with
/// "file not found".
pub fn file_path_to_string(file_path: FilePath) -> Result<Option<String>, String> {
    let Ok(path) = file_path.into_path() else {
        return Ok(None);
    };
    match path.to_str() {
        Some(value) => Ok(Some(value.to_string())),
        None => Err(format!(
            "The selected path contains characters AetherDesk cannot represent: {}",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn unicode_path_round_trips() {
        let path = PathBuf::from("C:\\giochi\\日本語\\Игра");
        let value = file_path_to_string(FilePath::Path(path.clone())).unwrap();
        assert_eq!(value, Some(path.to_str().unwrap().to_string()));
    }

    #[cfg(windows)]
    #[test]
    fn non_unicode_path_errors_instead_of_being_corrupted() {
        use std::os::windows::ffi::OsStringExt;

        let path = PathBuf::from(std::ffi::OsString::from_wide(&[0xD800, b'x' as u16]));
        let error = file_path_to_string(FilePath::Path(path)).unwrap_err();
        assert!(error.contains("cannot represent"), "got: {error}");
    }
}
