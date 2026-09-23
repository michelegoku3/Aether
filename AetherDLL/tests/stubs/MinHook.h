#pragma once
// Test double for registry synchronization/bookkeeping, NOT real hook tests.
#include <atomic>
enum MH_STATUS { MH_OK, MH_ERROR_ALREADY_INITIALIZED, MH_ERROR_ALREADY_CREATED, MH_ERROR_UNKNOWN };
inline constexpr auto MH_ALL_HOOKS = nullptr;
inline std::atomic<bool> testCreateFails{false};
inline MH_STATUS MH_Initialize() { return MH_OK; }
inline MH_STATUS MH_CreateHook(void*, void*, void**) { return testCreateFails ? MH_ERROR_UNKNOWN : MH_OK; }
inline MH_STATUS MH_EnableHook(void*) { return MH_OK; }
inline MH_STATUS MH_DisableHook(void*) { return MH_OK; }
inline MH_STATUS MH_Uninitialize() { return MH_OK; }
inline const char* MH_StatusToString(MH_STATUS) { return "test status"; }
