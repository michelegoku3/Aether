//! I-E: contract fixture DLL↔Desk per l'identità manifest (chiude D4/T2 sul
//! confine manifest).
//!
//! Gli STESSI blob binari in `Tools/contract_fixtures/manifest` (generati da
//! `Tools/contract_fixtures/generate_manifest_fixtures.py`) sono asseriti qui
//! e dalla suite C++ `manifest_contract` di quickwin_tests: se uno dei due
//! validatori cambia verdetto, uno dei due test si rompe. La tabella dei
//! verdetti attesi è in `Tools/contract_fixtures/manifest/TABLE.md`.

use crate::manifest::identity::{manifest_identity, verify_manifest_bytes};

const DEPOT: u32 = 489831;
const GID: u64 = 4940892828028256588;

fn fixture(name: &str) -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../Tools/contract_fixtures/manifest/"
    );
    let full = format!("{path}{name}");
    std::fs::read(&full).unwrap_or_else(|e| panic!("Fixture mancante {full}: {e}"))
}

#[test]
fn valid_fixtures_declare_the_expected_identity() {
    for name in ["valid_basic.bin", "valid_extra_wiretypes.bin"] {
        let bytes = fixture(name);
        assert_eq!(
            manifest_identity(&bytes).expect(name),
            (DEPOT, GID),
            "{name}"
        );
        // Raw bytes (non ZIP): l'unwrap è identity, i byte tornano identici.
        let verified = verify_manifest_bytes(&bytes, DEPOT, GID).expect(name);
        assert_eq!(verified, bytes, "{name}");
    }
}

#[test]
fn valid_fixture_rejects_wrong_expectations() {
    let bytes = fixture("valid_basic.bin");
    let error = verify_manifest_bytes(&bytes, DEPOT + 1, GID).unwrap_err();
    assert!(error.contains("identity mismatch"), "{error}");
    let error = verify_manifest_bytes(&bytes, DEPOT, GID + 1).unwrap_err();
    assert!(error.contains("identity mismatch"), "{error}");
}

#[test]
fn malformed_fixtures_are_rejected() {
    for name in [
        "bad_magic_payload.bin",
        "bad_magic_metadata.bin",
        "truncated_payload.bin",
        "truncated_metadata_varint.bin",
        "bad_wire_type.bin",
        "empty.bin",
        "garbage_short.bin",
    ] {
        let bytes = fixture(name);
        assert!(manifest_identity(&bytes).is_err(), "{name}");
        assert!(verify_manifest_bytes(&bytes, DEPOT, GID).is_err(), "{name}");
    }
}
