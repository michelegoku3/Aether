# ============================================================================
# AetherPayload - Payload Library (AetherPayload.dll)
#
# Online/EOS bridge payload.
# Injected into -onlinefix game processes by the main DLL.
# Hooks minimal EOS surface and self-propagates to child processes.
# ============================================================================

project(AetherPayload LANGUAGES CXX)

add_library(AetherPayload SHARED
    dllmain.cpp
    pch.cpp
    log/PayloadLog.cpp
    propagate/PayloadPropagator.cpp
    eos/EpicOnlineBridge.cpp
    util/RemoteDeploy.cpp
)

target_include_directories(AetherPayload PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_SOURCE_DIR}/common
)

# Enable Precompiled Headers
target_precompile_headers(AetherPayload PRIVATE
    $<$<COMPILE_LANGUAGE:CXX>:${CMAKE_SOURCE_DIR}/common/pch.h>
)

target_link_libraries(AetherPayload PRIVATE
    minhook
    Psapi
)

target_compile_definitions(AetherPayload PRIVATE
    WIN32_LEAN_AND_MEAN
    NOMINMAX
    _WIN32_WINNT=0x0A00
)

if(MSVC)
    target_compile_options(AetherPayload PRIVATE /utf-8 /EHsc)
endif()

set_target_properties(AetherPayload PROPERTIES
    OUTPUT_NAME "AetherPayload"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/out"
    RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/out/Release"
)