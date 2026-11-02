#pragma once

#include "framework.h"

namespace ac::hooks {

// Registers the ownership-related hooks on the live steamclient
// (CheckAppOwnership, LoadPackage, GetPackageInfo,
// MarkLicenseAsChanged, SendCallbackToPipe) and resolves the CUtlMemory grow
// helper.
//
// There is deliberately no GetSubscribedApps advertisement: apps reach the
// library through the package-0 license only, so Steam builds their AppInfo and
// local state before it renders a card. Appending ids to the subscribed list
// created overview entries without that bookkeeping and the client asserted.
void RegisterOwnershipHooks(HMODULE steamclient);

// Actively queries package 0 through the (already hooked) GetPackageInfo entry
// point. LoadPackage(package 0) fires exactly once, very early in the Steam
// process: when hook installation itself was delayed (e.g. by pattern
// downloads on a fresh build) that one-shot call is missed and pPackage0 is
// never published, stalling the startup seeding. Calling here reproduces what
// the missed LoadPackage hook would have done (publish pointer or seed).
// Returns true if package 0 was resolved and handled.
bool TryAcquirePackage0();

// Stops and joins the unlock-summary debounce thread. Safe to call when the
// hooks were never installed. Called from dllmain::Shutdown.
void ShutdownOwnershipHooks();

} // namespace ac::hooks
