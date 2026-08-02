#pragma once

#include <cstddef>
#include <cstdint>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// IpcReply — shared helpers for writing validated IPC reply shapes.
//
// Every IPC handler writes a fixed-layout reply into a CUtlBuffer that Steam
// provides (pWrite). The layouts are documented per-handler in CmdUser/CmdUtils
// and share a common tag byte (kIpcReplyTag) plus host-endian POD fields.
// These helpers centralize the three things every handler does by hand:
//   * capacity checks (never write past the buffer),
//   * the reply tag,
//   * POD field writes at explicit offsets.
//
// All functions are pure (no state) and operate only on the provided buffer, so
// this module has zero coupling to the rest of the IPC layer.
// ---------------------------------------------------------------------------
namespace ac::hooks::ipcreply {

// True when pWrite has room for at least `size` bytes starting at the reply tag
// (base[0]). No-op-safe: never writes.
bool CanWrite(steam::CUtlBuffer* pWrite, std::size_t size);

// Writes the reply tag (kIpcReplyTag) at base[0]. Returns false (no write) when
// the buffer has no room for at least 1 byte.
bool Begin(steam::CUtlBuffer* pWrite);

// Writes `bytes` from `data` at `offset` (from base). Returns false when the
// write would exceed the buffer capacity (nothing is written).
bool WriteAt(steam::CUtlBuffer* pWrite, std::size_t offset,
             const void* data, std::size_t bytes);

// Host-endian convenience wrappers over WriteAt.
bool WriteU32(steam::CUtlBuffer* pWrite, std::size_t offset, std::uint32_t value);
bool WriteU64(steam::CUtlBuffer* pWrite, std::size_t offset, std::uint64_t value);

}  // namespace ac::hooks::ipcreply
