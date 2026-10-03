cmake_minimum_required(VERSION 3.20)
project(AetherUtilityTests LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
include(FetchContent)
FetchContent_Declare(tomlplusplus
    GIT_REPOSITORY https://github.com/marzer/tomlplusplus.git
    GIT_TAG v3.4.0 GIT_SHALLOW TRUE)
FetchContent_MakeAvailable(tomlplusplus)
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
    ../AetherCore/hooks/ipc/IpcReply.cpp)
# The ABI modules keep their decision logic inline in the headers, so the
# cases "netpacket", "sentinel" and "structguard" test the real policy
# without linking the Windows glue (VirtualQuery, the PE walk, the logger).
# stubs/AbiReporting.cpp supplies the three reporting hooks those headers
# call; everything else they need is inline.
target_include_directories(quickwin_tests PRIVATE stubs ../AetherCore ../AetherPayload)
target_link_libraries(quickwin_tests PRIVATE Threads::Threads tomlplusplus::tomlplusplus)
if(MSVC)
    target_compile_options(quickwin_tests PRIVATE /W4 /utf-8 /EHsc)
else()
    target_compile_options(quickwin_tests PRIVATE -Wall -Wextra -Wpedantic)
endif()
foreach(case strings paths policy worker budget settings registry workers netpacket sentinel structguard gamesplayed_format donorpool keyvalues vdftext jsonutil_escapes manifest_identity ipcreply snapshot_merge)
    add_test(NAME ${case} COMMAND quickwin_tests ${case})
endforeach()
add_test(NAME cmake_source_inventory COMMAND ${CMAKE_COMMAND}
    -DTEST_BINARY_DIR=${CMAKE_CURRENT_BINARY_DIR}/inventory-test
    -DCHECK_MODULE=${CMAKE_CURRENT_SOURCE_DIR}/../cmake/CheckSources.cmake
    -P ${CMAKE_CURRENT_SOURCE_DIR}/source_inventory_test.cmake)
