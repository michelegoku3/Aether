#pragma once
#include "core/SteamTypes.h"
namespace ac::luadata {
inline bool HasDepot(steam::AppId appId) { return appId != 0; }
}
