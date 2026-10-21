//! `localconfig.vdf` carries every Steam client setting and every game's launch
//! options, and Steam rewrites it on exit. A commit must therefore be all or
//! nothing: the new text is staged in a sibling file and a rename replaces the
//! live one, so a crash, a kill or a full disk leaves the previous file whole.

use std::path::PathBuf;

const ORIGINAL: &str = r#""UserLocalConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"Apps"
				{
					"730"
					{
						"LaunchOptions"		"-novid"
					}
				}
			}
		}
	}
}
"#;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "aether_launch_options_tests_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Makes the destination impossible to replace while keeping it readable:
/// Windows refuses to rename over a read-only file. The original permissions
/// are returned so the file can be made writable again.
#[cfg(windows)]
fn block_replacement(path: &std::path::Path) -> std::fs::Permissions {
    let original = std::fs::metadata(path).unwrap().permissions();
    let mut blocked = original.clone();
    blocked.set_readonly(true);
    std::fs::set_permissions(path, blocked).unwrap();
    original
}

#[cfg(windows)]
fn allow_replacement(path: &std::path::Path, original: std::fs::Permissions) {
    std::fs::set_permissions(path, original).unwrap();
}

/// A commit that cannot replace the live file must leave it byte-identical and
/// must not leave a staging sibling behind.
#[cfg(windows)]
#[test]
fn a_failed_commit_keeps_the_original_and_leaves_no_tmp() {
    let dir = temp_dir("vdf-atomic");
    let path = dir.join("localconfig.vdf");
    std::fs::write(&path, ORIGINAL).unwrap();
    // The rename that publishes the new text cannot replace a read-only
    // destination, so the commit fails while the original is still intact.
    let original = block_replacement(&path);

    let err = crate::steam::launch_options::commit_localconfig(&path, "corrupt").unwrap_err();
    assert!(err.contains("localconfig.vdf"), "{err}");

    allow_replacement(&path, original);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), ORIGINAL);
    // The staging file belongs to the failed commit, so it is removed with it.
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "staging file left behind");
}

#[test]
fn commit_replaces_the_file_and_leaves_no_temp() {
    let dir = temp_dir("vdf-commit");
    let path = dir.join("localconfig.vdf");
    std::fs::write(&path, ORIGINAL).unwrap();

    crate::steam::launch_options::commit_localconfig(&path, "replacement").unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "replacement");
    // The backup is best effort and still holds the text before the commit.
    assert_eq!(
        std::fs::read_to_string(dir.join("localconfig.vdf.bak")).unwrap(),
        ORIGINAL
    );
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "staging file left behind");
}
