cmake_minimum_required(VERSION 3.20)
project(AetherUtilityTests LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
# Compilabile sia standalone (cmake -S tests) sia come add_subdirectory del
# progetto principale, dove tomlplusplus e' gia' stato fetchato.
if(NOT TARGET tomlplusplus::tomlplusplus)
    include(FetchContent)
    FetchContent_Declare(tomlplusplus
        GIT_REPOSITORY https://github.com/marzer/tomlplusplus.git
        GIT_TAG v3.4.0 GIT_SHALLOW TRUE)
    FetchContent_MakeAvailable(tomlplusplus)
endif()
find_package(Threads REQUIRED)
enable_testing()
add_executable(quickwin_tests quickwin_tests.cpp stubs/Services.cpp
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
target_include_directories(quickwin_tests PRIVATE stubs ../AetherCore ../AetherPayload)
# I-E: fixture binarie condivise con i test Rust di AetherDesk (D4).
target_compile_definitions(quickwin_tests PRIVATE
    CONTRACT_FIXTURES_DIR="${CMAKE_CURRENT_SOURCE_DIR}/../../Tools/contract_fixtures/manifest")
target_link_libraries(quickwin_tests PRIVATE Threads::Threads tomlplusplus::tomlplusplus)
if(MSVC)
    target_compile_options(quickwin_tests PRIVATE /W4 /WX /permissive- /utf-8 /EHsc /external:anglebrackets /external:W0)
else()
    target_compile_options(quickwin_tests PRIVATE -Wall -Wextra -Wpedantic -Werror)
endif()
foreach(case strings paths policy worker budget settings registry workers netpacket sentinel structguard gamesplayed_format donorpool keyvalues vdftext jsonutil_escapes manifest_identity ipcreply snapshot_merge ttlcache hexcodec signature ipcspec jsonwriter manifest_contract schema_buckets workers_race)
    add_test(NAME ${case} COMMAND quickwin_tests ${case})
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

set_tests_properties(workers workers_race PROPERTIES TIMEOUT 15)
