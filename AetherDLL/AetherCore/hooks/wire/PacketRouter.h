#pragma once

#include <cstdint>

#include "framework.h"
#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// Wire-level packet interception.
//
// Hooks the two steamclient networking entry points:
//   * BBuildAndAsyncSendFrame  — outgoing frames (client -> server)
//   * RecvPkt                  — incoming frames (server -> client)
//
// Each frame is decoded into (eMsg, header, body). Registered subsystems may
// rewrite the body (and, for recv, the header). To avoid heap churn on Steam's
// network thread, rewrites are assembled in static ring buffers.
//
// LumaCore packed every subsystem into one ~1000-line file. AetherCore keeps a
// thin core here and delegates to focused modules (AccessToken, FamilySharing,
// RichPresence), each exposing plain Handle* functions.
// ---------------------------------------------------------------------------
namespace ac::hooks {

// A decoded, immutable view of one frame passed to subsystem handlers.
struct WireFrame {
    std::uint32_t eMsg = 0;            // EMsg with the proto flag stripped
    const std::uint8_t* header = nullptr;
    std::uint32_t headerLen = 0;
    const std::uint8_t* body = nullptr;
    std::uint32_t bodyLen = 0;
};

// Subsystem handlers return the new serialized body length (written into a
// router-provided buffer), 0 to clear the body, or -1 to leave the frame
// unchanged. This keeps the handler contract simple and allocation-free.

// Installs the two network hooks (queued in the shared HookManager batch).
void RegisterPacketRouter(HMODULE diversion);

// ---------------------------------------------------------------------------
// Originate a client->CM protobuf frame from our own code.
//
// Reuses the connection object and the CMsgProtoBufHeader (steamid +
// client_sessionid) captured from an observed outbound frame, resetting the
// job ids so the message reads as a fresh unsolicited client notification.
// Used by PersonaInject to request AppInfo (PICS) records the local cache
// lacks, so masked friend sessions can render the real icon.
// Returns false when nothing has been captured yet, when the body is
// oversized, or when the send hook is not installed.
// ---------------------------------------------------------------------------
bool SendClientFrame(std::uint32_t eMsg, const std::uint8_t* body, std::uint32_t bodyLen);

}  // namespace ac::hooks
