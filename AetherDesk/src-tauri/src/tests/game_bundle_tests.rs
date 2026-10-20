//! Per-game export of the pinned Lua plus the exact `.manifest` files it
//! references.
//!
//! The export is an offline mirror of what AetherDLL loads for one game: the
//! Lua byte for byte and, for every pin, the exact `{depot}_{gid}.manifest`
//! that Steam reads. A manifest for another version of the same depot is not a
//! match — the pin is the identity — so it must stay missing.

use std::io::Read;

use crate::commands::manifests::{
    bundle_zip_name, collect_local_manifests, merge_generated_manifests, write_game_bundle,
    BundleManifest, BundleSource,
};
use crate::manifest::package::ManifestPackageFile;
use crate::manifest::pins::DepotManifestPin;
use crate::manifest::resolver::manifest_file_name;

fn pin(depot: u32, gid: &str) -> DepotManifestPin {
    DepotManifestPin {
        depot_id: depot,
        manifest_id: gid.to_string(),
    }
}

fn entry_bytes(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Vec<u8> {
    let mut entry = archive
        .by_name(name)
        .unwrap_or_else(|error| panic!("missing zip entry {name}: {error}"));
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).expect("read zip entry");
    bytes
}

#[test]
fn bundle_zip_name_sanitizes_characters_windows_rejects() {
    assert_eq!(
        bundle_zip_name("Halo: CE/Anniversary?", 1),
        "Halo CEAnniversary.zip"
    );
}

#[test]
fn bundle_zip_name_falls_back_to_the_app_id_when_nothing_is_left() {
    assert_eq!(bundle_zip_name("///", 42), "app_42.zip");
}

#[test]
fn collect_local_manifests_matches_the_exact_pin_only() {
    let steam = tempfile::tempdir().expect("temp steam root");
    let depotcache = steam.path().join("depotcache");
    let secondary = steam.path().join("config").join("depotcache");
    std::fs::create_dir_all(&depotcache).expect("depotcache");
    std::fs::create_dir_all(&secondary).expect("secondary depotcache");

    let primary = pin(111, "1001");
    let secondary_pin = pin(222, "2002");
    let other_version = pin(111, "9999");
    let empty = pin(333, "3003");
    std::fs::write(
        depotcache.join(manifest_file_name(&primary)),
        b"primary-bytes",
    )
    .expect("primary manifest");
    std::fs::write(
        secondary.join(manifest_file_name(&secondary_pin)),
        b"secondary-bytes",
    )
    .expect("secondary manifest");
    std::fs::write(
        depotcache.join(manifest_file_name(&pin(111, "7777"))),
        b"other-version",
    )
    .expect("other version");
    std::fs::write(depotcache.join(manifest_file_name(&empty)), b"").expect("empty manifest");

    let scan = collect_local_manifests(
        &steam.path().display().to_string(),
        987_654_321,
        &[primary, secondary_pin, other_version.clone(), empty.clone()],
    );

    assert_eq!(
        scan.local
            .iter()
            .map(|manifest| manifest.file_name.as_str())
            .collect::<Vec<_>>(),
        vec!["111_1001.manifest", "222_2002.manifest"],
        "only the exact pinned files are a match, and order follows the pins"
    );
    match &scan.local[0].source {
        BundleSource::Local(path) => {
            assert_eq!(
                path.as_path(),
                depotcache.join("111_1001.manifest").as_path()
            );
        }
        BundleSource::Generated(_) => panic!("a local pin must keep its on-disk path"),
    }
    assert_eq!(
        scan.missing,
        vec![other_version, empty],
        "another version of the same depot and an empty file are both missing"
    );
}

#[test]
fn merge_generated_manifests_fills_only_the_pins_hubcap_answered() {
    let generated_pin = pin(666, "6006");
    let still_missing = pin(777, "7007");
    let generated = vec![ManifestPackageFile {
        file_name: manifest_file_name(&generated_pin),
        bytes: vec![9, 9, 9],
    }];

    let (manifests, missing) =
        merge_generated_manifests(&[generated_pin, still_missing.clone()], &generated);

    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].file_name, "666_6006.manifest");
    match &manifests[0].source {
        BundleSource::Generated(bytes) => assert_eq!(bytes.as_slice(), &[9, 9, 9]),
        BundleSource::Local(_) => panic!("Hubcap bytes must be carried as generated"),
    }
    assert_eq!(missing, vec![still_missing]);
}

#[test]
fn write_game_bundle_contains_lua_and_manifests_and_overwrites() {
    let steam = tempfile::tempdir().expect("temp steam root");
    let downloads = tempfile::tempdir().expect("temp downloads");
    let local_manifest = steam.path().join("444_4004.manifest");
    std::fs::write(&local_manifest, b"local-bytes").expect("local manifest");

    let manifests = vec![
        BundleManifest {
            file_name: "444_4004.manifest".to_string(),
            source: BundleSource::Local(local_manifest),
        },
        BundleManifest {
            file_name: "555_5005.manifest".to_string(),
            source: BundleSource::Generated(b"generated-bytes".to_vec()),
        },
    ];
    let zip_name = bundle_zip_name("Test Game", 42);
    let lua = b"-- pinned lua\r\naddappid(444)\r\n";

    let first = write_game_bundle(downloads.path(), &zip_name, "42.lua", lua, &manifests)
        .expect("first export");
    let second = write_game_bundle(downloads.path(), &zip_name, "42.lua", lua, &manifests)
        .expect("second export");

    assert_eq!(
        first, second,
        "an existing bundle is replaced, not duplicated"
    );
    assert_eq!(
        first.file_name().and_then(|name| name.to_str()),
        Some("Test Game.zip")
    );
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(&second).expect("zip file")).expect("zip archive");
    assert_eq!(archive.len(), 3);
    assert_eq!(entry_bytes(&mut archive, "42.lua"), lua);
    assert_eq!(
        entry_bytes(&mut archive, "444_4004.manifest"),
        b"local-bytes"
    );
    assert_eq!(
        entry_bytes(&mut archive, "555_5005.manifest"),
        b"generated-bytes"
    );
}
