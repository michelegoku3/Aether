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

#[test]
fn failure_keeps_the_original_and_leaves_no_tmp() {
    let dir = temp_dir("vdf-atomic");
    let path = dir.join("localconfig.vdf");
    std::fs::write(&path, ORIGINAL).unwrap();
    // A sibling directory with the temp name cannot be created, so the temp
    // write fails while the original is still intact.
    let tmp_dir = dir.join(".localconfig.vdf.blocked");
    std::fs::create_dir(&tmp_dir).unwrap();
    let err =
        crate::steam::launch_options::commit_localconfig_with_temp(&path, "corrupt", &tmp_dir);
    assert!(err.is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), ORIGINAL);
    // A staging path this commit did not create belongs to someone else and is
    // never removed on failure.
    assert!(tmp_dir.is_dir());
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
