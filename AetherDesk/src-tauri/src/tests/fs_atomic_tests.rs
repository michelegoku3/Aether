//! `external_tools::fs::write_atomic` is the writer behind Lua backups, online
//! state and online preferences. These tests pin the guarantees every caller
//! relies on: a write never fails because another writer targets the same file,
//! and staging never leaves a `.tmp` sibling behind.

use std::path::PathBuf;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "aether_fs_atomic_tests_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Several domains write the same state file from different threads. A staging
/// name shared by every writer lets one writer rename another writer's file
/// away, so `Ok` and a whole payload are the only acceptable outcomes.
#[test]
fn concurrent_writes_of_the_same_target_all_succeed() {
    let dir = temp_dir("atomic-race");
    let target = dir.join("state.json");
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let target = target.clone();
            std::thread::spawn(move || {
                crate::external_tools::fs::write_atomic(
                    &target,
                    format!("{{\"n\":{i}}}").as_bytes(),
                )
            })
        })
        .collect();
    for h in handles {
        assert!(h.join().unwrap().is_ok());
    }
    let body = std::fs::read_to_string(&target).unwrap();
    assert!(body.starts_with("{\"n\":"));
    assert!(body.ends_with('}'));
    // The winning writer must have published its whole payload, not a prefix
    // left by an interleaved write.
    let inner = body
        .strip_prefix("{\"n\":")
        .and_then(|rest| rest.strip_suffix('}'))
        .expect("final file holds one complete payload");
    assert!(inner.parse::<u32>().is_ok(), "payload body: {inner:?}");
}

/// A staging file is owned by the writer that created it: a crashed or slower
/// writer may still hold `state.json.tmp`, so a write must neither reuse that
/// name nor publish its content. This is the deterministic form of the
/// concurrency race above.
#[test]
fn a_foreign_staging_file_is_neither_reused_nor_published() {
    let dir = temp_dir("atomic-stale-staging");
    let target = dir.join("state.json");
    let foreign = dir.join("state.json.tmp");
    std::fs::write(&foreign, b"someone-else's-bytes").unwrap();

    crate::external_tools::fs::write_atomic(&target, b"{\"mine\":true}").unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"{\"mine\":true}");
    assert_eq!(
        std::fs::read(&foreign).unwrap(),
        b"someone-else's-bytes",
        "a foreign staging file was reused or published"
    );
}

/// Staging files are working files, not state: a reader that lists the
/// directory must never find a `.tmp` sibling left by a successful write.
#[test]
fn no_tmp_sibling_is_left_behind() {
    let dir = temp_dir("atomic-leftover");
    let target = dir.join("state.json");
    crate::external_tools::fs::write_atomic(&target, b"{}").unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

/// A binary payload is copied through a staging file and renamed: the live
/// path is never truncated, and a failed copy leaves no sibling behind.
#[test]
fn copy_atomic_replaces_the_target_and_leaves_no_temp() {
    let dir = temp_dir("copy-atomic");
    let source = dir.join("source.bin");
    let target = dir.join("target.bin");
    std::fs::write(&source, b"new-bytes").unwrap();
    std::fs::write(&target, b"old-bytes").unwrap();

    crate::core::state_io::copy_atomic(&source, &target).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"new-bytes");
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "staging file left behind");
}
