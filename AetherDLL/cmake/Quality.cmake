# ============================================================================
# Quality gates per i target AetherDLL (AetherCore / AetherPayload / AetherXInput).
#
# Obiettivo: ogni build locale ("Compila tutto" in Visual Studio) e ogni build
# CI applicano GLI STESSI controlli, così un problema appare sulla macchina di
# chi lo introduce e non al rilascio.
#
#   aether_apply_quality(<target>)
#       - /W4 /permissive- /Zc:preprocessor su MSVC (prima: livello default W1)
#       - /WX (warning = errore) se AETHER_WARNINGS_AS_ERRORS=ON (default ON)
#       - clang-tidy sui sorgenti del target se AETHER_CLANG_TIDY != OFF e il
#         binario esiste (Visual Studio: componente "C++ Clang tools for
#         Windows"); configurazione in AetherDLL/.clang-tidy
#       - i sorgenti generati (protobuf) sono esclusi da lint e /WX
#
#   aether_check_headers(<target>)
#       - header orfani: ogni .h/.inl nella directory del target deve essere
#         incluso da almeno un sorgente/header del progetto. Complementa
#         aether_check_sources (che copre solo le translation unit).
#
# Escape hatch (mai silenzioso: viene stampato a configure):
#   cmake -DAETHER_WARNINGS_AS_ERRORS=OFF   -> warning visibili ma non bloccanti
#   cmake -DAETHER_CLANG_TIDY=OFF           -> salta clang-tidy
#   cmake -DAETHER_CLANG_TIDY=C:/path/clang-tidy.exe
# ============================================================================

option(AETHER_WARNINGS_AS_ERRORS "Treat compiler warnings as errors (/WX)" ON)
set(AETHER_CLANG_TIDY "AUTO" CACHE STRING "clang-tidy: AUTO | OFF | <path to clang-tidy>")

# Risolve il binario di clang-tidy una sola volta per configure.
if(NOT AETHER_CLANG_TIDY STREQUAL "OFF")
    if(AETHER_CLANG_TIDY STREQUAL "AUTO")
        # Percorsi tipici di Visual Studio 2022 oltre al PATH.
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
        # /W4            livello warning alto (default MSVC/CMake e' /W1-/W3)
        # /permissive-   C++ standard-conforme (niente estensioni silenziose MSVC)
        # /Zc:preprocessor preprocessore conforme (macro variadiche corrette)
        # /external:W0   zero warning dagli header di terze parti.
        # /external:I    MSVC considera "esterno" ogni header che sta SOTTO una
        #                di queste directory, indipendentemente da come e' stato
        #                incluso (-I normale, virgolette, PCH). Senza questo
        #                protobuf/abseil (FetchContent in _deps) e il .pb.h
        #                generato arrivano via -I e producono 60+ C4100 non
        #                azionabili.
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

    # Nota: i sorgenti protobuf generati NON passano di qui: stanno nel
    # target dedicato aether_proto (root CMakeLists) compilato con /w.

    if(AETHER_CLANG_TIDY_EXE)
        # CMake aggiunge da solo --driver-mode=cl quando il compilatore e' MSVC.
        # Il file .clang-tidy alla radice di AetherDLL governa i check.
        set_target_properties(${target} PROPERTIES
            CXX_CLANG_TIDY "${AETHER_CLANG_TIDY_EXE};--warnings-as-errors=*;--quiet")
    endif()
endfunction()

# Header orfani: un .h/.inl nella directory del target che nessun file del
# progetto include e' codice morto (o un refactor dimenticato a meta').
function(aether_check_headers target)
    file(GLOB_RECURSE _headers CONFIGURE_DEPENDS
        "${CMAKE_CURRENT_SOURCE_DIR}/*.h" "${CMAKE_CURRENT_SOURCE_DIR}/*.hpp"
        "${CMAKE_CURRENT_SOURCE_DIR}/*.inl")
    # Tutti i file che possono contenere #include: sorgenti e header di questo
    # target + la cartella common condivisa (pch.h/framework.h) + i test.
    file(GLOB_RECURSE _includers CONFIGURE_DEPENDS
        "${CMAKE_CURRENT_SOURCE_DIR}/*.cpp" "${CMAKE_CURRENT_SOURCE_DIR}/*.h"
        "${CMAKE_CURRENT_SOURCE_DIR}/*.inl"
        "${CMAKE_SOURCE_DIR}/common/*.h"
        "${CMAKE_SOURCE_DIR}/tests/*.cpp" "${CMAKE_SOURCE_DIR}/tests/*.h")

    # Raccoglie una sola volta tutte le direttive #include del progetto.
    set(_all_includes "")
    foreach(_file IN LISTS _includers)
        file(STRINGS "${_file}" _lines REGEX "^[ \t]*#[ \t]*include[ \t]*[<\"]")
        list(APPEND _all_includes ${_lines})
    endforeach()
    string(REPLACE ";" "\n" _all_includes_text "${_all_includes}")

    set(_orphans "")
    foreach(_hdr IN LISTS _headers)
        get_filename_component(_name "${_hdr}" NAME)
        # Match su "nome.h" preceduto da / o " o < : copre sia "core/X.h" che "X.h".
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
