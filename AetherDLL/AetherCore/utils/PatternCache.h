#pragma once

#include "core/AetherCoreState.h"
#include <string>

// Internal boundary: cache I/O builds an index; the engine publishes it.
namespace ac::pattern::detail {
using PatternIndex = AetherCoreState::PatternIndex;
void SweepTempFiles();
bool LoadModule(const std::string& moduleName, const std::string& dllPath, std::string& outSha, PatternIndex& outIndex);
} // namespace ac::pattern::detail
