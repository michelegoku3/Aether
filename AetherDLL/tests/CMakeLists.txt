cmake_minimum_required(VERSION 3.20)
project(AetherUtilityTests LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
# Buildable both standalone (cmake -S tests) and as an add_subdirectory of the
# main project, where tomlplusplus was already fetched.
if(NOT TARGET tomlplusplus::tomlplusplus)
    include(FetchContent)
    FetchContent_Declare(tomlplusplus
        GIT_REPOSITORY https://github.com/marzer/tomlplusplus.git
        GIT_TAG v3.4.0 GIT_SHALLOW TRUE)
    FetchContent_MakeAvailable(tomlplusplus)
endif()
find_package(Threads REQUIRED)
enable_testing()
# Core unit suites, one translation unit per area (see core_unit/): the old
# single quickwin_tests.cpp outgrew its name once it covered half the core.
add_executable(core_unit_tests
    core_unit/main.cpp
    core_unit/text.cpp
    core_unit/config.cpp
    core_unit/concurrency.cpp
    core_unit/registry.cpp
    core_unit/abi.cpp
    core_unit/wire.cpp
    core_unit/policy.cpp
    stubs/Services.cpp
    stubs/AbiReporting.cpp
    ../AetherCore/core/Settings.cpp ../AetherCore/core/HookManager.cpp
    ../AetherCore/core/Workers.cpp
    ../AetherCore/hooks/wire/GamesPlayedFormat.cpp
    ../AetherCore/hooks/wire/DonorPool.cpp
    ../AetherCore/utils/KeyValues.cpp
    ../AetherCore/network/ManifestIdentity.cpp
    ../AetherCore/hooks/ipc/IpcReply.cpp
    ../AetherCore/credentials/HexCodec.cpp
    ../AetherCore/utils/IpcSpecParse.cpp)
# The ABI modules keep their decision logic inline in the headers, so the
# cases "netpacket", "sentinel" and "structguard" test the real policy
# without linking the Windows glue (VirtualQuery, the PE walk, the logger).
# stubs/AbiReporting.cpp supplies the three reporting hooks those headers
# call; everything else they need is inline.
target_include_directories(core_unit_tests PRIVATE stubs ../AetherCore ../AetherPayload)
# Shared binary fixtures with the AetherDesk Rust tests (D4).
target_compile_definitions(core_unit_tests PRIVATE
    CONTRACT_FIXTURES_DIR="${CMAKE_CURRENT_SOURCE_DIR}/../../Tools/contract_fixtures/manifest")
target_link_libraries(core_unit_tests PRIVATE Threads::Threads tomlplusplus::tomlplusplus)
if(MSVC)
    target_compile_options(core_unit_tests PRIVATE /W4 /WX /permissive- /utf-8 /EHsc /external:anglebrackets /external:W0)
else()
    target_compile_options(core_unit_tests PRIVATE -Wall -Wextra -Wpedantic -Werror)
endif()
foreach(case strings paths policy worker budget settings registry workers netpacket sentinel structguard gamesplayed_format donorpool keyvalues vdftext jsonutil_escapes manifest_identity ipcreply snapshot_merge ttlcache hexcodec signature ipcspec jsonwriter manifest_contract schema_buckets workers_reap_slow workers_race)
    add_test(NAME ${case} COMMAND core_unit_tests ${case})
endforeach()
add_test(NAME cmake_source_inventory COMMAND ${CMAKE_COMMAND}
    -DTEST_BINARY_DIR=${CMAKE_CURRENT_BINARY_DIR}/inventory-test
    -DCHECK_MODULE=${CMAKE_CURRENT_SOURCE_DIR}/../cmake/CheckSources.cmake
    -P ${CMAKE_CURRENT_SOURCE_DIR}/source_inventory_test.cmake)

# Real backup queue + real worker registry, with disk/Steam dependencies stubbed.
add_executable(backup_lifecycle_tests backup_lifecycle_tests.cpp
    ../AetherCore/hooks/wire/AchievementBackup.cpp ../AetherCore/core/Workers.cpp)
target_include_directories(backup_lifecycle_tests PRIVATE backup_stubs ../AetherCore)
target_link_libraries(backup_lifecycle_tests PRIVATE Threads::Threads)
if(MSVC)
    target_compile_options(backup_lifecycle_tests PRIVATE /W4 /WX /permissive- /utf-8 /EHsc /external:anglebrackets /external:W0)
else()
    target_compile_options(backup_lifecycle_tests PRIVATE -Wall -Wextra -Wpedantic -Werror)
endif()
foreach(case registry_first flush_first concurrent job_failure registration_refused idle_registry startup_scan)
    add_test(NAME backup_${case} COMMAND backup_lifecycle_tests ${case})
    set_tests_properties(backup_${case} PROPERTIES TIMEOUT 10)
endforeach()

set_tests_properties(workers workers_reap_slow workers_race PROPERTIES TIMEOUT 15)

# Wire-contract tests: real ManifestRestore/BackupIo/Settings/Workers/Logger/
# RuntimeHttp against temp-dir fixtures, with only out-of-process collaborators
# stubbed (LicenseManager, ScriptEngine). Covers the shared copy contract
# (rules 2/3/4), unique staging, size-verify errors, pin scan, the Lua HTTPS
# gate, logger output and the remote-inject exit-code check.
add_executable(wire_contract_tests wire_contract_tests.cpp contract_stubs.cpp
    ../AetherCore/hooks/wire/ManifestRestore.cpp
    ../AetherCore/hooks/wire/BackupIo.cpp
    ../AetherCore/core/Settings.cpp
    ../AetherCore/core/Workers.cpp
    ../AetherCore/core/Logger.cpp
    ../AetherCore/network/HubcapQuota.cpp
    ../AetherCore/network/ManifestFetch.cpp
    ../AetherCore/network/ManifestIndex.cpp
    ../AetherCore/network/DepotCachePublisher.cpp
    ../AetherCore/network/ManifestIdentity.cpp
    ../AetherCore/security/ProviderCredentials.cpp
    ../AetherCore/network/RuntimeHttp.cpp)
target_include_directories(wire_contract_tests PRIVATE stubs ../AetherCore)
target_link_libraries(wire_contract_tests PRIVATE Threads::Threads tomlplusplus::tomlplusplus)
if(MSVC)
    target_compile_options(wire_contract_tests PRIVATE /W4 /WX /permissive- /utf-8 /EHsc /external:anglebrackets /external:W0)
else()
    target_compile_options(wire_contract_tests PRIVATE -Wall -Wextra -Wpedantic -Werror)
endif()
foreach(case staging_unique size_verify backup_rules backup_concurrent_writers pin_scan http_gate logger_golden logger_concurrent_burst logger_flush_drain quota_corrupt restore_roundtrip manifest_index_backup_hit unicode_paths remote_inject)
    add_test(NAME wire_${case} COMMAND wire_contract_tests ${case})
    set_tests_properties(wire_${case} PROPERTIES TIMEOUT 30)
endforeach()
