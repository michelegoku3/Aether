#pragma once

#include <string>

namespace ac::script {

// Creates the sandboxed Lua interpreter, registers bindings, and runs every
// .lua file in the configured directories. Returns false if Lua could not be
// initialised; a script error is logged but not fatal.
bool Init();

// Re-parses a single .lua file (hot-reload add/modify). Thread-safe with
// respect to the data layer; serialised against other ParseFile/UnloadFile
// calls by the watcher running them one at a time.
void ParseFile(const std::string& path);

// Drops a removed .lua file's depot references (hot-reload remove).
void UnloadFile(const std::string& path);

void Shutdown();

}  // namespace ac::script
