# ============================================================================
# AetherDLL - Professional Multi-Project Build Configuration
#
# Structure (clean professional layout):
#   - AetherCore/         : Main core DLL (AetherCore.dll)
#   - AetherPayload/      : Online/EOS payload DLL (AetherPayload.dll)
#   - AetherInjector/     : DWMAPI proxy injector (dwmapi.dll)
#
# Dependencies fetched via FetchContent (no manual vendoring required).
# Professional separation: each project is a self-contained subproject.
#
# Only Release builds are allowed.
#
# Build instructions:
#   cmake --preset x64-Release
#   cmake --build --preset x64-Release
# ============================================================================

cmake_minimum_required(VERSION 3.20)

# Policy for older FetchContent dependencies
set(CMAKE_POLICY_VERSION_MINIMUM 3.5 CACHE STRING "Floor for old dependency CMakeLists")

if(POLICY CMP0135)
    cmake_policy(SET CMP0135 NEW)
endif()

project(AetherDLL LANGUAGES C CXX VERSION 1.0.0)

if(NOT WIN32)
    message(FATAL_ERROR "AetherDLL targets Windows only.")
endif()
if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
    message(FATAL_ERROR "AetherDLL is x64-only. Select the x64 architecture/preset.")
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# Default build type
# Force Release only (no Debug builds allowed)
if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "" FORCE)
endif()
set(CMAKE_CONFIGURATION_TYPES "Release" CACHE STRING "" FORCE)

# Prevent accidental Debug configuration
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    message(FATAL_ERROR "Only Release builds are allowed for AetherDLL. Use the x64-Release preset.")
endif()

# ============================================================================
# Shared Dependencies (fetched once, used by all subprojects)
# ============================================================================
include(FetchContent)

# toml++
FetchContent_Declare(tomlplusplus
    GIT_REPOSITORY https://github.com/marzer/tomlplusplus.git
    GIT_TAG v3.4.0)
FetchContent_MakeAvailable(tomlplusplus)

# MinHook
FetchContent_Declare(minhook
    GIT_REPOSITORY https://github.com/TsudaKageyu/minhook.git
    GIT_TAG v1.3.4)
FetchContent_MakeAvailable(minhook)

# Lua 5.5
FetchContent_Declare(lua
    URL https://www.lua.org/ftp/lua-5.5.0.tar.gz)
FetchContent_MakeAvailable(lua)
file(GLOB LUA_SOURCES "${lua_SOURCE_DIR}/src/*.c")
list(REMOVE_ITEM LUA_SOURCES
    "${lua_SOURCE_DIR}/src/lua.c"
    "${lua_SOURCE_DIR}/src/luac.c")
add_library(lua_static STATIC ${LUA_SOURCES})
target_include_directories(lua_static PUBLIC "${lua_SOURCE_DIR}/src")

# Protobuf (v25.3 - stable for FetchContent on MSVC)
set(protobuf_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(protobuf_BUILD_CONFORMANCE OFF CACHE BOOL "" FORCE)
set(protobuf_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(protobuf_BUILD_PROTOC_BINARIES ON CACHE BOOL "" FORCE)
set(protobuf_BUILD_LIBUPB OFF CACHE BOOL "" FORCE)
set(protobuf_WITH_ZLIB OFF CACHE BOOL "" FORCE)
set(protobuf_MSVC_STATIC_RUNTIME OFF CACHE BOOL "" FORCE)
set(ABSL_PROPAGATE_CXX_STD ON CACHE BOOL "" FORCE)
set(ABSL_ENABLE_INSTALL ON CACHE BOOL "" FORCE)

FetchContent_Declare(protobuf
    GIT_REPOSITORY https://github.com/protocolbuffers/protobuf.git
    GIT_TAG v25.3)
FetchContent_MakeAvailable(protobuf)

set(AC_PROTOBUF_LITE_TARGET protobuf::libprotobuf-lite)
if(NOT TARGET protobuf::libprotobuf-lite AND TARGET libprotobuf-lite)
    set(AC_PROTOBUF_LITE_TARGET libprotobuf-lite)
endif()

# Proto generation (shared) - put generated files in a stable location
set(AC_PROTO_DIR "${CMAKE_BINARY_DIR}/generated")
file(MAKE_DIRECTORY "${AC_PROTO_DIR}")
set(AC_PROTO_SRC "${AC_PROTO_DIR}/steam_messages.pb.cc")
set(AC_PROTO_HDR "${AC_PROTO_DIR}/steam_messages.pb.h")

add_custom_command(
    OUTPUT "${AC_PROTO_SRC}" "${AC_PROTO_HDR}"
    COMMAND $<TARGET_FILE:protobuf::protoc>
            "--cpp_out=lite:${AC_PROTO_DIR}"
            -I "${CMAKE_CURRENT_SOURCE_DIR}/proto"
            "${CMAKE_CURRENT_SOURCE_DIR}/proto/steam_messages.proto"
    DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/proto/steam_messages.proto" protobuf::protoc
    COMMENT "Generating steam_messages.pb.{h,cc}"
)

# Make proto sources available to subprojects
add_custom_target(generate_protos DEPENDS "${AC_PROTO_SRC}" "${AC_PROTO_HDR}")

# ============================================================================
# Subprojects
# ============================================================================

add_subdirectory(AetherCore)
add_subdirectory(AetherPayload)
add_subdirectory(AetherInjector)

# ============================================================================
# Output Directory Configuration (common for all)
# ============================================================================
# All DLLs will land in ${CMAKE_BINARY_DIR}/out/<Config>/
set(OUTPUT_DIR "${CMAKE_BINARY_DIR}/out")

# ============================================================================
# Installation / Packaging (optional professional touch)
# ============================================================================
install(DIRECTORY ${OUTPUT_DIR}/ DESTINATION bin COMPONENT Runtime)
install(FILES README.md BUILD.md LICENSE DESTINATION .)