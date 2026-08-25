# ============================================================================
# AetherCore - Core Library (AetherCore.dll)
# Main hooking and logic core.
# ============================================================================

project(AetherCore LANGUAGES CXX)

add_library(AetherCore SHARED
    # Core
    core/dllmain.cpp
    core/AetherCoreState.cpp
    core/Logger.cpp
    core/Settings.cpp
    core/HookManager.cpp
    core/SteamVersion.cpp

    # Scripting
    scripting/ScriptEngine.cpp
    scripting/LuaSandbox.cpp
    scripting/LuaBindings.cpp
    scripting/LuaData.cpp
    scripting/DirWatch.cpp

    # Network
    network/RuntimeHttp.cpp
    network/ManifestFetch.cpp
    network/EticketFetcher.cpp

    # Credentials
    credentials/CredentialStore.cpp
    credentials/Ticket.cpp
    credentials/SteamId.cpp
    credentials/HexCodec.cpp

    # Utils
    utils/Hasher.cpp
    utils/EnvReader.cpp
    utils/PatternEngine.cpp
    utils/PatternDownloader.cpp
    utils/SmartIdLog.cpp
    utils/IpcSpec.cpp
    utils/GameNameResolver.cpp

    # Inject
    inject/Diversion.cpp

    # Diagnostics
    diagnostics/StatusWriter.cpp

    # Hooks
    hooks/steamui/SteamUIHook.cpp
    hooks/steamclient/OnlineFixHooks.cpp
    hooks/steamclient/OwnershipHooks.cpp
    hooks/steamclient/DepotHooks.cpp
    hooks/steamclient/DecryptionKeyHook.cpp
    hooks/steamclient/LicenseHooks.cpp
    hooks/license/LicenseManager.cpp
    hooks/ipc/IPCBus.cpp
    hooks/ipc/IpcReply.cpp
    hooks/ipc/CmdUser.cpp
    hooks/ipc/CmdUtils.cpp
    hooks/ipc/SteamCapture.cpp
    hooks/ipc/PipeWatch.cpp
    hooks/wire/PacketRouter.cpp
    hooks/wire/AchievementModule.cpp
    hooks/wire/AchievementBackup.cpp
    hooks/wire/BackupIo.cpp
    hooks/wire/UserStatsSnapshot.cpp
    hooks/wire/SteamStatsCache.cpp
    hooks/wire/PlaytimeMirror.cpp
    hooks/wire/AccessTokenModule.cpp
    hooks/wire/EticketModule.cpp
    hooks/wire/FamilySharingModule.cpp
    hooks/wire/GamesPlayedModule.cpp
    hooks/wire/PersonaInject.cpp
    hooks/wire/ManifestBridge.cpp
    hooks/wire/RichPresenceModule.cpp
    hooks/onlinefix/OnlinePayload.cpp
    hooks/onlinefix/CreateProcessHooks.cpp

    # Generated protobuf (location set by the root CMakeLists: AC_PROTO_DIR)
    ${AC_PROTO_DIR}/steam_messages.pb.cc

    # Precompiled header source
    pch.cpp
)

target_include_directories(AetherCore PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/..
    ${AC_PROTO_DIR}
    ${CMAKE_SOURCE_DIR}/common
)

# Enable Precompiled Headers
target_precompile_headers(AetherCore PRIVATE
    $<$<COMPILE_LANGUAGE:CXX>:${CMAKE_SOURCE_DIR}/common/pch.h>
)

target_link_libraries(AetherCore PRIVATE
    lua_static
    minhook
    tomlplusplus::tomlplusplus
    ${AC_PROTOBUF_LITE_TARGET}
    bcrypt
    winhttp
    shlwapi
    advapi32
    Psapi
)

target_compile_definitions(AetherCore PRIVATE
    WIN32_LEAN_AND_MEAN
    NOMINMAX
    _WIN32_WINNT=0x0A00
    AETHERCORE_RELEASE
)

if(MSVC)
    target_compile_options(AetherCore PRIVATE /bigobj /Zc:__cplusplus /utf-8 /EHsc)
endif()

set_target_properties(AetherCore PROPERTIES
    OUTPUT_NAME "AetherCore"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/out"
    RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/out/Release"
)

# Embed the Windows version resource (version comes from the root project VERSION)
attach_version_resource(AetherCore "AetherCore" "AetherCore.dll" "AetherDLL Core")

# Ensure proto generation is complete before building
add_dependencies(AetherCore generate_protos)