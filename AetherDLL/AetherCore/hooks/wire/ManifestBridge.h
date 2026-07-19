#pragma once

#include <cstdint>

#include "hooks/wire/PacketRouter.h"

// ---------------------------------------------------------------------------
// Manifest-request-code HTTP fallback.
//
// When Steam asks ContentServerDirectory.GetManifestRequestCode for a depot we
// faked ownership of, the server replies eresult != OK with an empty body and
// the download UI shows a bogus "no internet" error. This bridge fetches a
// request code from configured HTTP endpoints instead.
//
// Flow: on the outgoing 151 (send) we kick off an async fetch keyed on the
// job id; on the matching 147 (recv) we wait up to the configured timeout for
// the result and, if it arrives, rewrite the response header to OK and the body
// to carry the fetched code. Disabled when [manifest_fetch] urls is empty.
// ---------------------------------------------------------------------------
namespace ac::hooks::ManifestBridge {

// Outgoing GetManifestRequestCode service job. Returns -1 (never rewrites the
// request); it only records state for the recv side.
std::int32_t HandleSend(const WireFrame& frame);

// Incoming GetManifestRequestCode response. Returns the new body length written
// to out (and stamps a rewritten header via OutHeader), or -1 to pass through.
std::int32_t HandleRecv(const WireFrame& frame, std::uint8_t* out, std::uint32_t outCap,
                        std::uint8_t* outHeader, std::uint32_t outHeaderCap,
                        std::int32_t* outHeaderLen);

}  // namespace ac::hooks::ManifestBridge
