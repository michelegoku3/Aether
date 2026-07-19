#include "pch.h"
#include "EpicOnlineBridge.h"

#include <MinHook.h>

#include <atomic>
#include <cstddef>
#include <cstring>
#include <string>

#include "EpicOnlineTypes.h"
#include "../log/PayloadLog.h"

namespace ac::eosbridge {
namespace {

std::atomic_bool s_installed{false};
EOS_Connect_Login_t oLogin = nullptr;
EOS_Connect_CreateDeviceId_t oCreateDeviceId = nullptr;
EOS_IPOContainer_Add_t oIPOAdd = nullptr;
EOS_Lobby_OpFn_t oCreateLobby = nullptr;
EOS_Lobby_OpFn_t oJoinLobby = nullptr;
EOS_Lobby_OpFn_t oJoinLobbyById = nullptr;

struct LoginCtx {
    EOS_HConnect handle;
    EOS_Connect_OnLoginCb cb;
    void* cbData;
    std::string displayName;
};

std::string SteamPersonaName() {
    HMODULE sa = GetModuleHandleW(L"steam_api64.dll");
    if (!sa) sa = GetModuleHandleW(L"steam_api.dll");
    auto pFriends = sa ? reinterpret_cast<void* (*)()>(GetProcAddress(sa, "SteamFriends")) : nullptr;
    auto pName = sa ? reinterpret_cast<const char* (*)(void*)>(GetProcAddress(sa, "SteamAPI_ISteamFriends_GetPersonaName")) : nullptr;
    void* friends = pFriends ? pFriends() : nullptr;
    const char* name = (pName && friends) ? pName(friends) : nullptr;
    return (name && *name) ? name : "Unknown Player";
}

void OnLoginDone(const EOS_Connect_LoginCallbackInfo* info) {
    auto* ctx = static_cast<LoginCtx*>(info->ClientData);
    EOS_Connect_LoginCallbackInfo out = *info;
    out.ClientData = ctx->cbData;
    if (ctx->cb) ctx->cb(&out);
    delete ctx;
}

void OnCreateDeviceIdDone(const EOS_Connect_CreateDeviceIdCallbackInfo* info) {
    auto* ctx = static_cast<LoginCtx*>(info->ClientData);
    const bool ready = info->ResultCode == EOS_Success || info->ResultCode == EOS_DuplicateNotAllowed;
    if (!ready) {
        EOS_Connect_LoginCallbackInfo fail{};
        fail.ResultCode = info->ResultCode;
        fail.ClientData = ctx->cbData;
        if (ctx->cb) ctx->cb(&fail);
        delete ctx;
        return;
    }

    EOS_Connect_Credentials creds{1, nullptr, EOS_ECT_DEVICEID_ACCESS_TOKEN};
    EOS_Connect_UserLoginInfo who{1, ctx->displayName.c_str()};
    EOS_Connect_LoginOptions opts{2, &creds, &who};
    oLogin(ctx->handle, &opts, ctx, OnLoginDone);
}

void hkLogin(EOS_HConnect h, const EOS_Connect_LoginOptions*, void* cbData, EOS_Connect_OnLoginCb cb) {
    auto* ctx = new LoginCtx{h, cb, cbData, SteamPersonaName()};
    EOS_Connect_CreateDeviceIdOptions create{1, "PC"};
    oCreateDeviceId(h, &create, ctx, OnCreateDeviceIdDone);
}

EOS_EResult hkIPOAdd(EOS_HIntegratedPlatformOptionsContainer, const void*) {
    return EOS_Success;
}

void StripPresence(const void* opts, std::size_t flagOffset, std::int32_t minApiVer) {
    if (!opts) return;
    if (*reinterpret_cast<const std::int32_t*>(opts) < minApiVer) return;
    auto* flag = reinterpret_cast<EOS_Bool*>(reinterpret_cast<std::uintptr_t>(opts) + flagOffset);
    if (*flag) *flag = 0;
}

void hkCreateLobby(EOS_HLobby h, const void* opts, void* cd, void* cb) {
    StripPresence(opts, offsetof(EOS_Lobby_CreateLobbyOptions_Partial, bPresenceEnabled), 2);
    oCreateLobby(h, opts, cd, cb);
}
void hkJoinLobby(EOS_HLobby h, const void* opts, void* cd, void* cb) {
    StripPresence(opts, offsetof(EOS_Lobby_JoinLobbyOptions_Partial, bPresenceEnabled), 2);
    oJoinLobby(h, opts, cd, cb);
}
void hkJoinLobbyById(EOS_HLobby h, const void* opts, void* cd, void* cb) {
    StripPresence(opts, offsetof(EOS_Lobby_JoinLobbyByIdOptions_Partial, bPresenceEnabled), 1);
    oJoinLobbyById(h, opts, cd, cb);
}

template <typename Fn>
bool Resolve(HMODULE m, const char* name, Fn& slot) {
    slot = reinterpret_cast<Fn>(GetProcAddress(m, name));
    if (!slot) payloadlog::Write(std::string("Missing EOS export: ") + name);
    return slot != nullptr;
}

}  // namespace

void InstallOn(HMODULE eosModule) {
    bool expected = false;
    if (!eosModule || !s_installed.compare_exchange_strong(expected, true)) return;

    const auto init = MH_Initialize();
    if (init != MH_OK && init != MH_ERROR_ALREADY_INITIALIZED) {
        payloadlog::Write("MinHook initialization failed for EOS bridge.");
        s_installed.store(false);
        return;
    }

    bool ok = Resolve(eosModule, "EOS_Connect_Login", oLogin)
           && Resolve(eosModule, "EOS_Connect_CreateDeviceId", oCreateDeviceId)
           && Resolve(eosModule, "EOS_IntegratedPlatformOptionsContainer_Add", oIPOAdd)
           && Resolve(eosModule, "EOS_Lobby_CreateLobby", oCreateLobby)
           && Resolve(eosModule, "EOS_Lobby_JoinLobby", oJoinLobby)
           && Resolve(eosModule, "EOS_Lobby_JoinLobbyById", oJoinLobbyById);
    if (!ok) {
        payloadlog::Write("One or more EOS exports missing; EOS bridge disabled.");
        s_installed.store(false);
        return;
    }

    if (MH_CreateHook(oLogin, hkLogin, reinterpret_cast<void**>(&oLogin)) != MH_OK ||
        MH_CreateHook(oIPOAdd, hkIPOAdd, reinterpret_cast<void**>(&oIPOAdd)) != MH_OK ||
        MH_CreateHook(oCreateLobby, hkCreateLobby, reinterpret_cast<void**>(&oCreateLobby)) != MH_OK ||
        MH_CreateHook(oJoinLobby, hkJoinLobby, reinterpret_cast<void**>(&oJoinLobby)) != MH_OK ||
        MH_CreateHook(oJoinLobbyById, hkJoinLobbyById, reinterpret_cast<void**>(&oJoinLobbyById)) != MH_OK) {
        payloadlog::Write("EOS hook creation failed.");
        s_installed.store(false);
        return;
    }

    MH_EnableHook(MH_ALL_HOOKS);
    payloadlog::Write("EOS bridge hooks installed successfully.");
}

}  // namespace ac::eosbridge
