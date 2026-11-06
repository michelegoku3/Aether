import { useEffect } from 'react';
import { getSettings, type AppSettings } from '../hooks/useSettings';

interface Params {
  loadDeskVersion: () => Promise<void>;
  refreshAppearance: (settings?: AppSettings) => Promise<void>;
  refreshHubcapUsage: (forcedKey: undefined, settings: AppSettings) => Promise<void>;
  checkDeskUpdates: () => Promise<void>;
  checkDllUpdates: (settings: AppSettings) => Promise<void>;
  warnIfSteamPathMissing: () => Promise<void>;
}

/** Performs the one-shot startup hydration with a single shared settings read. */
export function useAppBootstrap(params: Params) {
  useEffect(() => {
    const hydrate = async () => {
      // Startup consumers that do not depend on settings run regardless of the
      // settings outcome, so a failed read never delays the version checks.
      // The library warm-up is NOT here: it used to rescan the whole library
      // before the provider's own scan. It now runs once after that scan.
      const deskVersionPromise = params.loadDeskVersion();
      // The Steam-path warning only needs the settings file and is the only
      // route to fixing a missing path: it must not queue behind the network
      // update checks, which can sit on a slow or silent connection. The catch
      // is attached here because the await is far away: a rejection must not
      // surface as unhandled while the checks run.
      const steamPathWarning = params.warnIfSteamPathMissing().catch((error) => {
        console.warn('[startup] Steam-path check failed:', error);
      });

      let settings: AppSettings | null = null;
      try {
        settings = await getSettings();
      } catch (error) {
        // A corrupt settings.json or a DPAPI failure must not freeze startup:
        // refreshAppearance() re-reads, falls back to defaults and still marks
        // settings ready, so the rest of the app stays usable.
        console.warn('Settings load failed at startup:', error);
      }

      if (settings) {
        await Promise.all([
          params.refreshAppearance(settings),
          params.refreshHubcapUsage(undefined, settings),
          params.checkDllUpdates(settings),
          // The Desk dot is the one the user waits for, and its check does not
          // read settings on the frontend side: running it here keeps it out of
          // the queue behind the DLL lookup and the Hubcap quota.
          params.checkDeskUpdates(),
        ]);
      } else {
        await Promise.all([params.refreshAppearance(), params.checkDeskUpdates()]);
      }

      await Promise.all([steamPathWarning, deskVersionPromise]);
    };
    void hydrate();
    // App remains mounted for its lifetime; every callback is a stable hook API.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
