#pragma once

#include "core/AetherCoreState.h"
#include <string>

// Internal boundary: cache I/O builds an index; the engine publishes it.
namespace ac::pattern::detail {
using PatternIndex = AetherCoreState::PatternIndex;
void SweepTempFiles();
bool LoadModule(const std::string& moduleName, const std::string& dllPath, std::string& outSha, PatternIndex& outIndex);
// Checks upstream for a newer generation or preferred source of an already
// cached table and updates the on-disk cache for future reloads/launches.
// Returns true if the on-disk cache was updated.
bool RefreshCachedModule(const std::string& moduleName, const std::string& dllPath);
} // namespace ac::pattern::detail
