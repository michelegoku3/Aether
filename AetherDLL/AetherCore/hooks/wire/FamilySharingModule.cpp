#include "pch.h"
#include "hooks/wire/FamilySharingModule.h"

#include "core/Logger.h"
#include "scripting/LuaData.h"

namespace ac::hooks::FamilySharing {

bool ShouldSuppress() {
    // Family notifications are global and do not carry a stable app ID in all
    // Steam builds. Use Aether's managed-data source as the conservative gate
    // instead of guessing fields from an unstable protobuf body.
    return !luadata::AllDepotIds().empty();
}

std::int32_t ClearBody() {
    AC_LOG_DEBUG_ONCE("Wire.Family", "Clearing family-sharing notification body.");
    return 0;
}

}  // namespace ac::hooks::FamilySharing
