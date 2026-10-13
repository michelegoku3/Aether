# ============================================================================
# Quality gates for the AetherDLL targets (AetherCore / AetherPayload / AetherXInput).
#
# Goal: every local build ("Build All" in Visual Studio) and every CI build
# apply THE SAME checks, so a problem shows up on its author's machine, not at
# release time.
#
#   aether_apply_quality(<target>)
#       - /W4 /permissive- /Zc:preprocessor on MSVC (before: default level W1)
#       - /WX (warning = error) when AETHER_WARNINGS_AS_ERRORS=ON (default ON)
#       - clang-tidy on the target's sources when AETHER_CLANG_TIDY != OFF and
#         the binary exists (Visual Studio: "C++ Clang tools for Windows"
#         component); configuration in AetherDLL/.clang-tidy
#       - generated sources (protobuf) are excluded from lint and /WX
#
#   aether_check_headers(<target>)
#       - orphan headers: every .h/.inl in the target directory must be
#         included by at least one project source/header. Complements
#         aether_check_sources (which covers translation units only).
#
# Escape hatch (never silent: printed at configure time):
#   cmake -DAETHER_WARNINGS_AS_ERRORS=OFF   -> visible but non-blocking warnings
#   cmake -DAETHER_CLANG_TIDY=OFF           -> skips clang-tidy
#   cmake -DAETHER_CLANG_TIDY=C:/path/clang-tidy.exe
# ============================================================================

option(AETHER_WARNINGS_AS_ERRORS "Treat compiler warnings as errors (/WX)" ON)
set(AETHER_CLANG_TIDY "AUTO" CACHE STRING "clang-tidy: AUTO | OFF | <path to clang-tidy>")

# Resolves the clang-tidy binary once per configure.
if(NOT AETHER_CLANG_TIDY STREQUAL "OFF")
    if(AETHER_CLANG_TIDY STREQUAL "AUTO")
        # Typical Visual Studio 2022 paths beyond PATH.
        file(GLOB _aether_vs_llvm
            "C:/Program Files/Microsoft Visual Studio/2022/*/VC/Tools/Llvm/x64/bin"
            "C:/Program Files/Microsoft Visual Studio/2022/*/VC/Tools/Llvm/bin"
            "C:/Program Files/LLVM/bin")
        find_program(AETHER_CLANG_TIDY_EXE NAMES clang-tidy HINTS ${_aether_vs_llvm})
    else()
        set(AETHER_CLANG_TIDY_EXE "${AETHER_CLANG_TIDY}" CACHE FILEPATH "" FORCE)
    endif()
    if(AETHER_CLANG_TIDY_EXE)
        message(STATUS "[AetherDLL] clang-tidy enabled: ${AETHER_CLANG_TIDY_EXE}")
    else()
        message(STATUS "[AetherDLL] clang-tidy NOT found: static analysis skipped "
                       "(install 'C++ Clang tools for Windows' in the VS Installer, "
                       "or pass -DAETHER_CLANG_TIDY=<path>)")
    endif()
else()
    message(STATUS "[AetherDLL] clang-tidy disabled by AETHER_CLANG_TIDY=OFF")
endif()

if(AETHER_WARNINGS_AS_ERRORS)
    message(STATUS "[AetherDLL] Warnings are errors (/WX). Disable with -DAETHER_WARNINGS_AS_ERRORS=OFF")
else()
    message(WARNING "[AetherDLL] AETHER_WARNINGS_AS_ERRORS=OFF: warnings will NOT fail the build")
endif()

function(aether_apply_quality target)
    if(MSVC)
        # /W4            high warning level (MSVC/CMake default is /W1-/W3)
        # /permissive-   standards-conforming C++ (no silent MSVC extensions)
        # /Zc:preprocessor conforming preprocessor (correct variadic macros)
        # /external:W0   zero warnings from third-party headers.
        # /external:I    MSVC treats as "external" any header sitting UNDER one
        #                of these directories, regardless of how it was included
        #                (plain -I, quotes, PCH). Without this, protobuf/abseil
        #                (FetchContent in _deps) and the generated .pb.h arrive
        #                via -I and produce 60+ unactionable C4100s.
        target_compile_options(${target} PRIVATE
            /W4 /permissive- /Zc:preprocessor
            /external:anglebrackets /external:W0
            "/external:I${CMAKE_BINARY_DIR}/_deps"
            "/external:I${CMAKE_SOURCE_DIR}/proto/generated"
            "/external:I${CMAKE_BINARY_DIR}/generated")   # AETHER_PROTO_PREGEN=OFF
        if(AETHER_WARNINGS_AS_ERRORS)
            target_compile_options(${target} PRIVATE /WX)
        endif()
    else()
        target_compile_options(${target} PRIVATE -Wall -Wextra -Wpedantic)
        if(AETHER_WARNINGS_AS_ERRORS)
            target_compile_options(${target} PRIVATE -Werror)
        endif()
    endif()

    # Note: generated protobuf sources do NOT go through here: they live in the
    # dedicated aether_proto target (root CMakeLists) compiled with /w.

    if(AETHER_CLANG_TIDY_EXE)
        # CMake adds --driver-mode=cl on its own when the compiler is MSVC.
        # The .clang-tidy file at the AetherDLL root governs the checks.
        set_target_properties(${target} PROPERTIES
            CXX_CLANG_TIDY "${AETHER_CLANG_TIDY_EXE};--warnings-as-errors=*;--quiet")
    endif()
endfunction()

# Orphan headers: an .h/.inl in the target directory that no project file
# includes is dead code (or a refactor forgotten halfway).
function(aether_check_headers target)
    file(GLOB_RECURSE _headers CONFIGURE_DEPENDS
        "${CMAKE_CURRENT_SOURCE_DIR}/*.h" "${CMAKE_CURRENT_SOURCE_DIR}/*.hpp"
        "${CMAKE_CURRENT_SOURCE_DIR}/*.inl")
    # Every file that may hold #include: this target's sources and headers
    # + the shared common folder (pch.h/framework.h) + the tests.
    file(GLOB_RECURSE _includers CONFIGURE_DEPENDS
        "${CMAKE_CURRENT_SOURCE_DIR}/*.cpp" "${CMAKE_CURRENT_SOURCE_DIR}/*.h"
        "${CMAKE_CURRENT_SOURCE_DIR}/*.inl"
        "${CMAKE_SOURCE_DIR}/common/*.h"
        "${CMAKE_SOURCE_DIR}/tests/*.cpp" "${CMAKE_SOURCE_DIR}/tests/*.h")

    # Collects every #include directive of the project exactly once.
    set(_all_includes "")
    foreach(_file IN LISTS _includers)
        file(STRINGS "${_file}" _lines REGEX "^[ \t]*#[ \t]*include[ \t]*[<\"]")
        list(APPEND _all_includes ${_lines})
    endforeach()
    string(REPLACE ";" "\n" _all_includes_text "${_all_includes}")

    set(_orphans "")
    foreach(_hdr IN LISTS _headers)
        get_filename_component(_name "${_hdr}" NAME)
        # Match on "name.h" preceded by / or " or < : covers both "core/X.h" and "X.h".
        string(REGEX MATCH "[/\"<]${_name}[\">]" _hit "${_all_includes_text}")
        if(NOT _hit)
            list(APPEND _orphans "${_hdr}")
        endif()
    endforeach()

    if(_orphans)
        string(REPLACE ";" "\n  " _orphans_text "${_orphans}")
        message(FATAL_ERROR "[${target}] Orphan header(s) never included by any source:\n  ${_orphans_text}\nRemove them, or include them where they are actually needed.")
    endif()
    message(STATUS "[${target}] Header inventory checked: no orphan headers")
endfunction()
