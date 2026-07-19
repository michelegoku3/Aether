#pragma once

#include "utils/PatternEngine.h"

namespace ac {

template <typename Fn>
bool HookManager::TryHook(const std::string& name, const std::string& module, HMODULE hModule,
                          Fn& original, Fn detour) {
    void* target = pattern::ResolveAddress(name, module, hModule);
    if (!target) {
        RecordMissed(name);
        return false;
    }
    RegisterHook(name, target, reinterpret_cast<void**>(&original), reinterpret_cast<void*>(detour));
    return true;
}

}  // namespace ac
