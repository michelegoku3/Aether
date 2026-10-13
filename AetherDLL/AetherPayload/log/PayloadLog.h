#pragma once

#include <string_view>
#include <windows.h>

namespace ac::payloadlog {

void Init(HMODULE self);
void Write(std::string_view line);

} // namespace ac::payloadlog
