import { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getSettings, type AppSettings } from '../hooks/useSettings';
import type { DllStatusInfo } from '../types/ui';

const NOT_INSTALLED: DllStatusInfo = {
  isInstalled: false,
  installedVersion: 'N/A',
  isSteamBlocked: false,
};

/** Owns the installed/blocked/version snapshot shown by the Aether panel. */
export function useDllRuntime() {
  const [status, setStatus] = useState<DllStatusInfo>(NOT_INSTALLED);

  const checkStatus = useCallback(async (preloadedSettings?: AppSettings) => {
    try {
      const settings = preloadedSettings ?? (await getSettings());
      if (!settings.steam_path?.trim()) return;

      // Status is a local read. The network update check belongs to
      // `useUpdateCoordinator` alone, so startup does not ask GitHub twice.
      const [isInstalled, isBlocked, installedVersion] = await Promise.all([
        invoke<boolean>('is_dll_installed'),
        invoke<boolean>('is_steam_blocked'),
        invoke<string>('get_installed_dll_version'),
      ]);
      setStatus({
        isInstalled,
        installedVersion: installedVersion || 'N/A',
        isSteamBlocked: isBlocked,
      });
    } catch (error) {
      console.error('Failed to check DLL status:', error);
      setStatus(NOT_INSTALLED);
    }
  }, []);

  return { status, checkStatus };
}
