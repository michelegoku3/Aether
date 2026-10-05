#!/usr/bin/env python3
"""I-E: generatore deterministico delle contract fixture DLL<->Desk per il
validatore di identità manifest (D4).

Gli STESSI blob binari vengono consumati da:
  * AetherDLL/tests/quickwin_tests.cpp   (suite "manifest_contract")
  * AetherDesk/src-tauri/src/tests/manifest_contract_tests.rs

Se cambia il formato, si rigenera qui e la tabella in TABLE.md deve essere
aggiornata di conseguenza: i test su entrambi i lati asseriscono gli stessi
verdetti.
"""
import os
import struct

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "manifest")

PAYLOAD_MAGIC = 0x71F617D0
METADATA_MAGIC = 0x1F4812BE

DEPOT = 489831
GID = 4940892828028256588  # stesso valore usato dai test storici di Desk


def varint(value: int) -> bytes:
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value == 0:
            out.append(byte)
            return bytes(out)
        out.append(byte | 0x80)


def frame(payload: bytes, metadata: bytes) -> bytes:
    out = bytearray()
    out += struct.pack("<I", PAYLOAD_MAGIC)
    out += struct.pack("<I", len(payload))
    out += payload
    out += struct.pack("<I", METADATA_MAGIC)
    out += struct.pack("<I", len(metadata))
    out += metadata
    return bytes(out)


def meta_basic(depot: int = DEPOT, gid: int = GID) -> bytes:
    """campo 1 (depot) + campo 2 (gid) + un campo length-delimited da skippare."""
    m = bytearray()
    m += varint((1 << 3) | 0) + varint(depot)
    m += varint((2 << 3) | 0) + varint(gid)
    m += varint((7 << 3) | 2) + varint(3) + b"abc"
    return bytes(m)


def meta_extra_wiretypes() -> bytes:
    """Tutti i wire type supportati (0, 1, 2, 5) su campi sconosciuti."""
    m = bytearray()
    m += varint((1 << 3) | 0) + varint(DEPOT)
    m += varint((3 << 3) | 1) + b"\x01" * 8          # wire 1: 64-bit
    m += varint((4 << 3) | 5) + b"\x02\x02\x03\x04"  # wire 5: 32-bit
    m += varint((7 << 3) | 2) + varint(2) + b"xy"    # wire 2: bytes
    m += varint((2 << 3) | 0) + varint(GID)
    m += varint((9 << 3) | 0) + varint(42)           # altro varint
    return bytes(m)


FIXTURES = {}

# --- validi: identità leggibile da ENTRAMBI i lati -------------------------
FIXTURES["valid_basic.bin"] = frame(b"payload-bytes", meta_basic())
FIXTURES["valid_extra_wiretypes.bin"] = frame(b"payload-2", meta_extra_wiretypes())

# --- malformati: entrambi i lati devono rifiutare ---------------------------
bad = bytearray(frame(b"payload-bytes", meta_basic()))
bad[0] ^= 0xFF  # rompe la magia del payload
FIXTURES["bad_magic_payload.bin"] = bytes(bad)

bad = bytearray(frame(b"payload-bytes", meta_basic()))
bad[8 + len(b"payload-bytes")] ^= 0xFF  # rompe la magia del metadata
FIXTURES["bad_magic_metadata.bin"] = bytes(bad)

# payload_len dichiara più byte di quanti ne esistono
FIXTURES["truncated_payload.bin"] = frame(b"payload-bytes", meta_basic())[:14]

# metadata troncato a metà di un varint (campo gid: 0x80... sospeso)
full = frame(b"payload-bytes", meta_basic())
FIXTURES["truncated_metadata_varint.bin"] = full[: len(full) - 6]

# wire type 3 (gruppo, non supportato) nel metadata
bad_meta = varint((1 << 3) | 0) + varint(DEPOT) + varint((2 << 3) | 3) + b"\x00"
FIXTURES["bad_wire_type.bin"] = frame(b"payload-bytes", bad_meta)

FIXTURES["empty.bin"] = b""
FIXTURES["garbage_short.bin"] = b"\x01\x02\x03\x04\x05\x06\x07"

TABLE = """# Contract fixture — identità manifest (D4 / I-E)

Blob binari sintetici, generati da `generate_manifest_fixtures.py`, consumati
IDENTICI dai test C++ (quickwin_tests, suite `manifest_contract`) e Rust
(`manifest_contract_tests.rs`). `depot` atteso = {depot}, `gid` atteso = {gid}.

| file | verdetto atteso (entrambi i lati) |
|---|---|
| valid_basic.bin | identità = ({depot}, {gid}); match con attesi; mismatch con attesi diversi |
| valid_extra_wiretypes.bin | identità = ({depot}, {gid}) (wire type 0/1/2/5 skippati) |
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
""".format(depot=DEPOT, gid=GID)


def main() -> None:
    os.makedirs(OUT, exist_ok=True)
    for name, blob in FIXTURES.items():
        with open(os.path.join(OUT, name), "wb") as f:
            f.write(blob)
        print(f"{name}: {len(blob)} byte")
    with open(os.path.join(OUT, "TABLE.md"), "w", encoding="utf-8") as f:
        f.write(TABLE)
    print("TABLE.md scritta")


if __name__ == "__main__":
    main()
