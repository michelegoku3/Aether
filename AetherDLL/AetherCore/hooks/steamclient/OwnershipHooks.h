#pragma once

#include <cstdint>

#include "framework.h"
#include "core/SteamTypes.h"

namespace ac::hooks {

// Registers the ownership-related hooks on the live steamclient
// (CheckAppOwnership, LoadPackage, GetPackageInfo,
// MarkLicenseAsChanged, ProcessPendingLicenseUpdates, SendCallbackToPipe).
//
// There is deliberately no GetSubscribedApps advertisement: apps reach the
// library through the package-0 license only, so Steam builds their AppInfo and
// local state before it renders a card. Appending ids to the subscribed list
// created overview entries without that bookkeeping and the client asserted.
void RegisterOwnershipHooks(HMODULE steamclient);

// Actively queries package 0 through the (already hooked) GetPackageInfo
// trampoline, returning its PackageInfo* if available. Must be called ONLY on
// the steamclient main thread (`CSteamEngine::BMainLoop`), never concurrently
// with CUser::RunFrame.
steam::PackageInfo* AcquirePackage0OnSteamThread();

// Calls the real MarkLicenseAsChanged / ProcessPendingLicenseUpdates trampolines
// on behalf of LicenseManager. Must be called ONLY on the steamclient main thread.
bool MarkLicenseAsChangedOnSteamThread(void* cUser, std::uint32_t packageId, bool reloadAll);
bool ProcessPendingLicenseUpdatesOnSteamThread(void* cUser);
bool HasLicenseRefreshHelpers() noexcept;

// Stops and joins the unlock-summary debounce thread. Safe to call when the
// hooks were never installed. Called from dllmain::Shutdown.
void ShutdownOwnershipHooks();

} // namespace ac::hooks
