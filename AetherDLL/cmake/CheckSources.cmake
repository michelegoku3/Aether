# Verify actual target membership (not a textual grep of CMakeLists).
function(aether_check_sources target)
    get_target_property(sources ${target} SOURCES)
    set(compiled)
    foreach(source IN LISTS sources)
        get_filename_component(full "${source}" ABSOLUTE BASE_DIR "${CMAKE_CURRENT_SOURCE_DIR}")
        list(APPEND compiled "${full}")
    endforeach()
    file(GLOB_RECURSE on_disk CONFIGURE_DEPENDS
        "${CMAKE_CURRENT_SOURCE_DIR}/*.cpp" "${CMAKE_CURRENT_SOURCE_DIR}/*.cc"
        "${CMAKE_CURRENT_SOURCE_DIR}/*.cxx" "${CMAKE_CURRENT_SOURCE_DIR}/*.c")
    foreach(source IN LISTS on_disk)
        if(NOT source IN_LIST compiled)
            message(FATAL_ERROR "[${target}] Uncompiled source: ${source}. Add it to the target or remove/archive it outside the target directory.")
        endif()
    endforeach()
    message(STATUS "[${target}] Source inventory checked: no orphan translation units")
endfunction()
