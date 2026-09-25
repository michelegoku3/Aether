# Steam bootstrap: the xinput1_4.dll proxy forwards named and private ordinal
# exports to the system DLL. Exports are defined in xinput1_4.def.
project(AetherXInput LANGUAGES CXX)

add_library(AetherXInput SHARED
    dllmain.cpp
    xinput1_4.def
)

target_compile_definitions(AetherXInput PRIVATE
    WIN32_LEAN_AND_MEAN
    NOMINMAX
    _WIN32_WINNT=0x0A00
)
if(MSVC)
    target_compile_options(AetherXInput PRIVATE /utf-8 /EHsc)
endif()

set_target_properties(AetherXInput PROPERTIES
    OUTPUT_NAME "xinput1_4"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/out"
    RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/out/Release"
)

attach_version_resource(AetherXInput "AetherXInput" "xinput1_4.dll" "AetherDLL XInput Proxy")
aether_check_sources(AetherXInput)
