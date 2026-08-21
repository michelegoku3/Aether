use std::path::Path;

use crate::local::{classify_local_file, LocalFileKind};

#[test]
fn lua_and_manifest_extensions_are_counted_independently() {
    assert_eq!(classify_local_file(Path::new("1158310.lua")), LocalFileKind::Lua);
    assert_eq!(
        classify_local_file(Path::new("1158311_123456.manifest")),
        LocalFileKind::Manifest
    );
}

#[test]
fn extension_matching_is_case_insensitive_and_exact() {
    assert_eq!(classify_local_file(Path::new("GAME.LUA")), LocalFileKind::Lua);
    assert_eq!(classify_local_file(Path::new("DEPOT.MANIFEST")), LocalFileKind::Manifest);
    assert_eq!(
        classify_local_file(Path::new("not-a-manifest.manifest.bak")),
        LocalFileKind::GameFile
    );
}
