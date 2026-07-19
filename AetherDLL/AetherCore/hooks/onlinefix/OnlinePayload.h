#pragma once

#include <cstdint>
#include <string>

#include "hooks/ipc/PipeWatch.h"

namespace ac::hooks::onlinepayload {

void MaybeInject(const pipewatch::ProcessSnapshot& snapshot);
std::size_t InjectedPidCount();

}  // namespace ac::hooks::onlinepayload
