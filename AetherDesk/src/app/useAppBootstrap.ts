import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getSettings, type AppSettings } from '../hooks/useSettings';

interface Params {
  loadDeskVersion: () => Promise<void>;
  refreshAppearance: (settings: AppSettings) => Promise<void>;
  refreshHubcapUsage: (forcedKey: undefined, settings: AppSettings) => Promise<void>;
  checkDeskUpdates: () => Promise<void>;
  checkDllUpdates: (settings: AppSettings) => Promise<void>;
  warnIfSteamPathMissing: () => Promise<void>;
}

/** Performs the one-shot startup hydration with a single shared settings read. */
export function useAppBootstrap(params: Params) {
  useEffect(() => {
    const hydrate = async () => {
      const deskVersionPromise = params.loadDeskVersion();
      const warmPromise = invoke<number>('warm_library_game_cache')
        .then((count) => console.log(
          `[AetherDesk library cache warm-up] ${count} cached names available`,
        ))
        .catch((error) => console.warn('Library cache warm-up failed:', error));
      const settingsPromise = getSettings();
      const [settings] = await Promise.all([
        settingsPromise,
        deskVersionPromise,
        warmPromise,
      ]);
      await Promise.all([
        params.refreshAppearance(settings),
        params.refreshHubcapUsage(undefined, settings),
        params.checkDeskUpdates(),
        params.checkDllUpdates(settings),
        params.warnIfSteamPathMissing(),
      ]);
    };
    void hydrate();
    // App remains mounted for its lifetime; every callback is a stable hook API.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
