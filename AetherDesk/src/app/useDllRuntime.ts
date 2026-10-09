import { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getSettings, type AppSettings } from '../hooks/useSettings';
import type { DllStatusInfo } from '../types/ui';
import type { DllUpdateInfo } from './types';

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
      const settings = preloadedSettings ?? await getSettings();
      if (!settings.steam_path?.trim()) return;

      const [isInstalled, isBlocked, updateInfo] = await Promise.all([
        invoke<boolean>('is_dll_installed'),
        invoke<boolean>('is_steam_blocked'),
        invoke<DllUpdateInfo>('check_aether_dll_update'),
      ]);
      setStatus({
        isInstalled,
        installedVersion: updateInfo.installed_version || 'N/A',
        isSteamBlocked: isBlocked,
      });
    } catch (error) {
      console.error('Failed to check DLL status:', error);
      setStatus(NOT_INSTALLED);
    }
  }, []);

  return { status, checkStatus };
}
