#pragma once

// ---------------------------------------------------------------------------
// Pure parser for the IPC spec TOML.
//
// Extracted from IpcSpec.cpp::ParseToml: the same parsing logic, but without
// touching g_state or the filesystem, so the "ipcspec" suite in quickwin_tests
// can verify it in isolation. IpcSpec.cpp stays the only consumer publishing
// the result into g_state.ipcSpec.
// ---------------------------------------------------------------------------

#include <cstdint>
#include <string>
#include <unordered_map>

#include "utils/IpcSpec.h"

namespace ac::ipcspec {

// Parses an IPC spec TOML document into the output maps. Returns false when
// the document is not valid TOML or yields not even one valid method (same
// semantics as before: an empty/unparsable file must NOT overwrite the
// existing state — the caller publishes only on true).
bool ParseSpecToml(const std::string& body, std::unordered_map<std::string, std::uint8_t>& outInterfaceIds,
                   std::unordered_map<std::string, MethodSpec>& outMethods);

} // namespace ac::ipcspec
