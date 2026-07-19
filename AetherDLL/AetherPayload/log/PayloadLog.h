#pragma once

#include <windows.h>
#include <string_view>

namespace ac::payloadlog {

void Init(HMODULE self);
void Write(std::string_view line);

}  // namespace ac::payloadlog
