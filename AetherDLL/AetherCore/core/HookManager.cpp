#include "pch.h"
#include "core/HookManager.h"

#include <MinHook.h>

#include <sstream>

#include "core/Logger.h"

namespace ac {
namespace {
constexpr const char* kModule = "HookManager";
}

void HookManager::RegisterHook(const std::string& name, void* target, void** original, void* detour) {
    hooks_.push_back(HookInfo{name, target, original, detour, false});
}

void HookManager::RecordMissed(const std::string& name) {
    AC_LOG_WARN(kModule, "Hook '%s' missed: pattern not resolved.", name.c_str());
    diag::Record("hook_miss", name);
    missed_.push_back(name);
}

bool HookManager::InstallAll() {
    MH_STATUS init = MH_Initialize();
    if (init != MH_OK && init != MH_ERROR_ALREADY_INITIALIZED) {
        AC_LOG_ERROR(kModule, "MH_Initialize failed: %s", MH_StatusToString(init));
        return false;
    }

    for (auto& hook : hooks_) {
        if (hook.created) continue;
        MH_STATUS status = MH_CreateHook(hook.target, hook.detour, hook.original);
        if (status == MH_OK || status == MH_ERROR_ALREADY_CREATED) {
            hook.created = true;
            ++installedCount_;
            installed_.push_back(hook.name);
            diag::Record("hook_installed", hook.name);
        } else {
            // A single failed hook must not abort the rest (graceful degradation).
            AC_LOG_ERROR(kModule, "Hook '%s' creation failed: %s",
                         hook.name.c_str(), MH_StatusToString(status));
            diag::Record("hook_create_failed", hook.name);
            missed_.push_back(hook.name);
        }
    }

    MH_STATUS enable = MH_EnableHook(MH_ALL_HOOKS);
    if (enable != MH_OK) {
        AC_LOG_ERROR(kModule, "MH_EnableHook failed: %s", MH_StatusToString(enable));
        return false;
    }
    std::ostringstream installed;
    for (std::size_t i = 0; i < installed_.size(); ++i) {
        if (i) installed << ", ";
        installed << installed_[i];
    }

    if (missed_.empty()) {
        AC_LOG_INFO(kModule, "Enabled %d hooks (0 missed): [%s].", installedCount_,
                    installed.str().c_str());
    } else {
        std::ostringstream missed;
        for (std::size_t i = 0; i < missed_.size(); ++i) {
            if (i) missed << ", ";
            missed << missed_[i];
        }
        AC_LOG_WARN(kModule, "Enabled %d hooks (%zu missed): installed=[%s] missed=[%s].",
                    installedCount_, missed_.size(), installed.str().c_str(),
                    missed.str().c_str());
    }
    return true;
}

bool HookManager::UninstallAll() {
    MH_DisableHook(MH_ALL_HOOKS);
    MH_Uninitialize();
    for (auto& hook : hooks_) hook.created = false;
    installed_.clear();
    installedCount_ = 0;
    return true;
}

}  // namespace ac
