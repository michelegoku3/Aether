# Contract fixture — identità manifest (D4 / I-E)

Blob binari sintetici, generati da `generate_manifest_fixtures.py`, consumati
IDENTICI dai test C++ (quickwin_tests, suite `manifest_contract`) e Rust
(`manifest_contract_tests.rs`). `depot` atteso = 489831, `gid` atteso = 4940892828028256588.

| file | verdetto atteso (entrambi i lati) |
|---|---|
| valid_basic.bin | identità = (489831, 4940892828028256588); match con attesi; mismatch con attesi diversi |
| valid_extra_wiretypes.bin | identità = (489831, 4940892828028256588) (wire type 0/1/2/5 skippati) |
| bad_magic_payload.bin | rifiutato |
| bad_magic_metadata.bin | rifiutato |
| truncated_payload.bin | rifiutato |
| truncated_metadata_varint.bin | rifiutato |
| bad_wire_type.bin | rifiutato |
| empty.bin | rifiutato |
| garbage_short.bin | rifiutato |

Nota: un manifest senza campo depot/gid dichiara identità (0,0). Il C++
confronta con gli attesi (match solo se attesi 0), il Rust rifiuta a priori
identità zero — divergenza INTENZIONALE documentata, non coperta da questa
tabella perché i verdetti differiscono per progetto.
