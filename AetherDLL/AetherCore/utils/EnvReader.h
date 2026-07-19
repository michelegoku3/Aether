#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// EnvReader — read the environment block of a remote Windows process and
// resolve Steam-specific variables into AppIds.
//
// Works by walking the remote PEB (x64 and WOW64), reading the process
// parameters, and parsing the double-null-terminated environment block.
//
// Stateless and thread-safe: operates solely on the caller-provided HANDLE.
// No globals, no mutex, no persistent state.  Every failure path returns
// nullopt cleanly — callers fall back to their own logic (architectural
// principle 3: graceful degradation).
//
// Why this module exists (architectural audit 2026-07-12):
//   PipeWatch originally carried this logic inline (~156 lines out of 400).
//   Extracting it keeps PipeWatch focused on IPC handshake orchestration and
//   makes environment reading reusable by future modules (e.g. Denuvo auth,
//   ProcessExtension) without duplication — a problem LumaCore suffered from.
// ---------------------------------------------------------------------------
namespace ac::env {

// Resolved Steam environment variables from a remote process.
struct EnvAppIds {
    steam::AppId steamAppId = 0;
    steam::AppId steamGameId = 0;
    steam::AppId steamOverlayGameId = 0;
    steam::AppId selected = 0;        // Best resolution: OverlayGameId > SteamGameId > SteamAppId
    const char* source = "fallback";  // Which variable produced "selected"
};

// Reads the full environment block from a remote process.
//
// On success returns a vector of wchar_t containing the double-null-terminated
// block trimmed to the last meaningful entry.  On any failure (process
// inaccessible, PEB unreadable, no double-null terminator found) returns
// nullopt.
//
// Building block: exposed so callers can inspect non-Steam variables without
// reimplementing PEB traversal.
std::optional<std::vector<wchar_t>> ReadEnvironmentBlock(HANDLE process);

// Reads and resolves the three Steam environment variables.
//
// Resolution order (first wins):
//   1. SteamOverlayGameId  (decoded as GameID → low 24 bits)
//   2. SteamGameId         (decoded as GameID → low 24 bits)
//   3. SteamAppId          (direct app id)
//
// Returns nullopt when the process cannot be read or none of the three
// variables is present.
std::optional<EnvAppIds> ReadSteamEnvAppIds(HANDLE process);

}  // namespace ac::env
