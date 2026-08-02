#pragma once

#include <cstdint>

#include "core/SteamTypes.h"

// ---------------------------------------------------------------------------
// IPC bus interception.
//
// Hooks steamclient!IPCProcessMessage, parses the InterfaceCall header, and
// dispatches to a registered handler for (interfaceId, funcHash). Handlers may
// overwrite the response buffer to spoof ownership/ticket replies for apps we
// are configured to handle.
//
// Steam-internal pipes are filtered out, and handlers only run for app ids that
// have a configured depot, so unrelated traffic stays byte-identical.
// ---------------------------------------------------------------------------
namespace ac::hooks {

// A handler runs after Steam's original message processing succeeds. It reads
// the request from pRead and writes a spoofed reply into pWrite.
using IpcHandlerFn = void (*)(steam::CSteamPipeClient* pipe,
                              steam::CUtlBuffer* pRead,
                              steam::CUtlBuffer* pWrite);

struct IpcHandlerEntry {
    std::uint8_t interfaceId;
    std::uint32_t funcHash;
    const char* name;
    IpcHandlerFn handler;
};

// Adds handlers to the dispatch table. Called by CmdUser/CmdUtils before the
// hook is installed.
void RegisterIpcHandlers(const IpcHandlerEntry* entries, std::size_t count);

// Resolves GetPipeClient, registers the command modules, and installs the
// IPCProcessMessage hook (queued in the shared HookManager batch).
void RegisterIpcBus(HMODULE diversion);

// ---------------------------------------------------------------------------
// Contract for adding a new IPC interface (e.g. IClientFriends):
//   1. Create hooks/ipc/CmdXxx.{h,cpp} with `void Register();`.
//   2. In CmdXxx.cpp, define a `kEntries[]` of IpcHandlerEntry with
//      (ipc_iface::kXxx, ipc_hash::kXxx_Method, "IXxx::Method", handler).
//   3. Call `CmdXxx::Register()` from RegisterIpcBus() (in IPCBus.cpp) BEFORE
//      the IPCProcessMessage hook is queued.
//   4. Write replies through hooks/ipc/IpcReply.h (validated shape helpers)
//      so every response keeps the same tag/layout contract with Steam.
// Handlers run only for pipes whose app is Lua-configured (see h_IPCProcessMessage).
// ---------------------------------------------------------------------------
}  // namespace ac::hooks
