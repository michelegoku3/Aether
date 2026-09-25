#pragma once

namespace ac {

// Prepare the chosen steamclient hook target (copy or live). Auto mode attaches
// live if Steam has already mapped its client; otherwise it prepares a copy.
bool LoadDiversion();

// Auto mode's final decision, after pattern downloads and just before the
// steamui redirect is installed. Must run only once, before hook registration.
void SelectHookTargetBeforeRedirect();

}  // namespace ac
