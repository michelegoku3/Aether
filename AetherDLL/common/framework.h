#pragma once

// ============================================================================
// framework.h - Shared Windows configuration header
//
// This file is included by all three projects (AetherCore, AetherPayload,
// AetherInjector) to ensure consistent Windows header configuration.
//
// Key settings:
//   - WIN32_LEAN_AND_MEAN : Reduces the size of windows.h by excluding
//                           rarely used services.
//   - NOMINMAX            : Prevents windows.h from defining min/max macros
//                           that conflict with std::min/std::max and protobuf.
// ============================================================================

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif

#ifndef NOMINMAX
#define NOMINMAX
#endif

#include <windows.h>