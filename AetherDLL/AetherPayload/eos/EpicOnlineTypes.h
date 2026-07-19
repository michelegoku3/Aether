#pragma once

#include <cstdint>

using EOS_EResult = std::int32_t;
using EOS_Bool = std::int32_t;
using EOS_HConnect = void*;
using EOS_HLobby = void*;
using EOS_HIntegratedPlatformOptionsContainer = void*;
using EOS_ProductUserId = void*;
using EOS_ContinuanceToken = void*;

constexpr EOS_EResult EOS_Success = 0;
constexpr EOS_EResult EOS_DuplicateNotAllowed = 24;
constexpr std::int32_t EOS_ECT_DEVICEID_ACCESS_TOKEN = 10;

#pragma pack(push, 8)
struct EOS_Connect_Credentials {
    std::int32_t ApiVersion;
    const char* Token;
    std::int32_t Type;
};
struct EOS_Connect_UserLoginInfo {
    std::int32_t ApiVersion;
    const char* DisplayName;
};
struct EOS_Connect_LoginOptions {
    std::int32_t ApiVersion;
    const EOS_Connect_Credentials* Credentials;
    const EOS_Connect_UserLoginInfo* UserLoginInfo;
};
struct EOS_Connect_LoginCallbackInfo {
    EOS_EResult ResultCode;
    void* ClientData;
    EOS_ProductUserId LocalUserId;
    EOS_ContinuanceToken ContinuanceToken;
};
struct EOS_Connect_CreateDeviceIdOptions {
    std::int32_t ApiVersion;
    const char* DeviceModel;
};
struct EOS_Connect_CreateDeviceIdCallbackInfo {
    EOS_EResult ResultCode;
    void* ClientData;
};
struct EOS_Lobby_CreateLobbyOptions_Partial {
    std::int32_t ApiVersion;
    EOS_ProductUserId LocalUserId;
    std::uint32_t MaxLobbyMembers;
    std::int32_t PermissionLevel;
    EOS_Bool bPresenceEnabled;
};
struct EOS_Lobby_JoinLobbyOptions_Partial {
    std::int32_t ApiVersion;
    void* LobbyDetailsHandle;
    EOS_ProductUserId LocalUserId;
    EOS_Bool bPresenceEnabled;
};
struct EOS_Lobby_JoinLobbyByIdOptions_Partial {
    std::int32_t ApiVersion;
    const char* LobbyId;
    EOS_ProductUserId LocalUserId;
    EOS_Bool bPresenceEnabled;
};
#pragma pack(pop)

using EOS_Connect_OnLoginCb = void(*)(const EOS_Connect_LoginCallbackInfo*);
using EOS_Connect_OnCreateDeviceIdCb = void(*)(const EOS_Connect_CreateDeviceIdCallbackInfo*);
using EOS_Connect_Login_t = void(*)(EOS_HConnect, const EOS_Connect_LoginOptions*, void*, EOS_Connect_OnLoginCb);
using EOS_Connect_CreateDeviceId_t = void(*)(EOS_HConnect, const EOS_Connect_CreateDeviceIdOptions*, void*, EOS_Connect_OnCreateDeviceIdCb);
using EOS_IPOContainer_Add_t = EOS_EResult(*)(EOS_HIntegratedPlatformOptionsContainer, const void*);
using EOS_Lobby_OpFn_t = void(*)(EOS_HLobby, const void*, void*, void*);
