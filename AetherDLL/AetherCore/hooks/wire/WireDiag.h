#pragma once

#include <cstdint>

#include "hooks/wire/PacketRouter.h"

// ---------------------------------------------------------------------------
// WireDiag — human-readable wire diagnostics (flight-recorder support).
//
// Maps eresult codes to names/classes for logs and status.json, and emits
// the bidirectional per-frame trace (one frame = one line, so the log's
// last line is ALWAYS the last frame before a crash). No hook logic here.
// ---------------------------------------------------------------------------
namespace ac::hooks {

const char* EresultName(std::int32_t eresult);
const char* EresultClass(std::int32_t eresult);
void TraceFrame(const char* dir, const WireFrame& f);

} // namespace ac::hooks
