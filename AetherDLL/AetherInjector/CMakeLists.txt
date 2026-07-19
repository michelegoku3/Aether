# ============================================================================
# AetherInjector - DWMAPI Proxy Injector (dwmapi.dll)
#
# A tiny standalone DLL that Steam loads at startup.
# Forwards every dwmapi export to the system original and loads AetherDLL.dll
# into steam.exe.
#
# Output name MUST be "dwmapi" to impersonate the system DLL.
# ============================================================================

project(AetherInjector LANGUAGES CXX)

add_library(AetherInjector SHARED
    dllmain.cpp
)

target_include_directories(AetherInjector PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_SOURCE_DIR}/common
)

# Enable Precompiled Headers (opzionale per progetto piccolo)
target_precompile_headers(AetherInjector PRIVATE
    $<$<COMPILE_LANGUAGE:CXX>:${CMAKE_SOURCE_DIR}/common/pch.h>
)

target_compile_definitions(AetherInjector PRIVATE
    WIN32_LEAN_AND_MEAN
    NOMINMAX
    _WIN32_WINNT=0x0A00
)

target_link_libraries(AetherInjector PRIVATE
    shlwapi
)

set_target_properties(AetherInjector PROPERTIES
    OUTPUT_NAME "dwmapi"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/out"
    RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/out/Release"
)