#pragma once

#include <string>
#include "core/AetherCoreState.h"

// Internal boundary: cache I/O builds an index; the engine publishes it.
namespace ac::pattern::detail {
using PatternIndex = AetherCoreState::PatternIndex;
void SweepTempFiles();
bool LoadModule(const std::string& moduleName, const std::string& dllPath,
                std::string& outSha, PatternIndex& outIndex);
}  // namespace ac::pattern::detail
