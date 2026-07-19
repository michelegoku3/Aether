#pragma once

#include <string>

// ---------------------------------------------------------------------------
// Address resolution backed by per-build TOML pattern files.
//
// Each Steam DLL is identified by its SHA-256. A matching <sha>.toml maps
// function names to an RVA (and optional verification signature). The file is
// loaded from the local cache or downloaded on demand. If no pattern is
// available the affected hooks are simply skipped (graceful degradation).
// ---------------------------------------------------------------------------
namespace ac::pattern {

// Computes module SHAs, then loads/downloads the pattern tables for
// steamclient and steamui. Returns true if at least one table loaded.
bool Init();

// Resolves a function address inside hModule. module is "steamclient" or
// "steamui". Returns nullptr if the entry is missing or its signature no
// longer matches (the function moved) — callers must treat null as "skip".
void* ResolveAddress(const std::string& funcName, const std::string& module, HMODULE hModule);

}  // namespace ac::pattern
