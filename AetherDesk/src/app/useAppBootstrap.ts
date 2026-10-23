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
        ]);
      } else {
        await params.refreshAppearance();
      }

      await Promise.all([params.checkDeskUpdates(), params.warnIfSteamPathMissing(), deskVersionPromise]);
    };
    void hydrate();
    // App remains mounted for its lifetime; every callback is a stable hook API.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
