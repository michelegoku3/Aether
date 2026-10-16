#pragma once

#include <cstdint>
#include <string>

#include "core/SteamTypes.h"
#include "hooks/wire/PacketRouter.h"

// ---------------------------------------------------------------------------
// ServiceJobTracker — jobid <-> service-name correlation for the wire layer.
//
// Service calls (151/146) carry target_job_name + jobid_source; their
// responses (147) carry only jobid_target. This module decodes request
// headers once (single ParseFromArray per frame) and remembers the mapping
// so responses can be attributed. Pure dispatch support: no hooks here.
// ---------------------------------------------------------------------------
namespace ac::hooks {

// Decoded request header: name and/or source id from one parse.
struct ServiceRequest {
    std::string name;
    std::uint64_t source = 0;
    bool hasName = false;
    bool hasSource = false;
};

// Decoded response header: optional name plus correlation metadata.
struct ServiceResponse {
    std::string name;
    std::uint64_t target = 0;
    std::int32_t eresult = -1;
    bool hasName = false;
    bool hasTarget = false;
};

bool ParseServiceRequest(const WireFrame& f, ServiceRequest& out);
bool ParseServiceResponse(const WireFrame& f, ServiceResponse& out);

// Flight-recorder correlation: remember id->name on send, consume on recv
// (consuming keeps the map bounded: every resolve drops its entry).
void TrackServiceJob(std::uint64_t id, std::string name);
std::string ResolveServiceJob(std::uint64_t jobIdTarget);

} // namespace ac::hooks
