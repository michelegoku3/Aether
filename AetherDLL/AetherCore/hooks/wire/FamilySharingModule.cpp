#include "pch.h"
#include "hooks/wire/FamilySharingModule.h"

#include "core/Logger.h"

namespace ac::hooks::FamilySharing {

std::int32_t ClearBody() {
    AC_LOG_DEBUG_ONCE("Wire.Family", "Clearing family-sharing notification body.");
    return 0;
}

}  // namespace ac::hooks::FamilySharing
