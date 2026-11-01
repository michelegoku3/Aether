import { useCallback, useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { getSettings, type AppSettings } from '../hooks/useSettings';
import type { DeskUpdateInfo, DllUpdateInfo } from './types';

interface AutoUpdateReadyPayload {
  tag_name: string;
  install_root: string;
}

/** Owns Desk/DLL update availability, the installed Desk version label, and
 * the background auto-update restart prompt. */
export function useUpdateCoordinator() {
  const [dllUpdateAvailable, setDllUpdateAvailable] = useState(false);
  const [deskVersion, setDeskVersion] = useState('…');
  const [deskUpdateAvailable, setDeskUpdateAvailable] = useState(false);
  const [deskUpdateIsTest, setDeskUpdateIsTest] = useState(false);
  const [dllUpdateIsTest, setDllUpdateIsTest] = useState(false);
  const [autoUpdateTag, setAutoUpdateTag] = useState<string | null>(null);

  useEffect(() => {
    // The background checker emits this when it has staged a newer release
    // and is waiting for the user to restart. Unlisten on unmount.
    let unlisten: (() => void) | undefined;
    listen<AutoUpdateReadyPayload>('aetherdesk://auto-update-ready', (event) => {
      setAutoUpdateTag(event.payload.tag_name ?? null);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  const dismissAutoUpdate = useCallback(() => {
    setAutoUpdateTag(null);
  }, []);

  const restartForAutoUpdate = useCallback(async () => {
    setAutoUpdateTag(null);
    await invoke('install_aether_desk_update');
  }, []);

  const loadDeskVersion = useCallback(async () => {
    try {
      const version = await invoke<string>('get_desk_version');
      setDeskVersion(version || 'N/A');
    } catch {
      setDeskVersion('N/A');
    }
  }, []);

  const checkDeskUpdates = useCallback(async () => {
    try {
      const info = await invoke<DeskUpdateInfo>('check_aether_desk_update');
      console.log('[AetherDesk update check]', info);
      setDeskUpdateAvailable(Boolean(info.update_available));
      setDeskUpdateIsTest(Boolean(info.is_test));
      if (info.installed_version) setDeskVersion(info.installed_version);
    } catch (error) {
      console.error('AetherDesk update check failed:', error);
      setDeskUpdateAvailable(false);
      setDeskUpdateIsTest(false);
    }
  }, []);

  const checkDllUpdates = useCallback(async (settings: AppSettings) => {
    try {
      if (settings.steam_path?.trim()) {
        const info = await invoke<DllUpdateInfo>('check_aether_dll_update');
        console.log('[AetherDLL update check]', info);
        setDllUpdateAvailable(Boolean(info.update_available));
        setDllUpdateIsTest(Boolean(info.is_test));
      } else {
        setDllUpdateAvailable(false);
        setDllUpdateIsTest(false);
      }
    } catch (error) {
      console.error('AetherDLL update check failed:', error);
      setDllUpdateAvailable(false);
      setDllUpdateIsTest(false);
    }
  }, []);

  const checkAllUpdates = useCallback(async () => {
    const settings = await getSettings();
    await Promise.all([checkDeskUpdates(), checkDllUpdates(settings)]);
  }, [checkDeskUpdates, checkDllUpdates]);

  return {
    dllUpdateAvailable,
    deskVersion,
    deskUpdateAvailable,
    deskUpdateIsTest,
    dllUpdateIsTest,
    autoUpdateTag,
    dismissAutoUpdate,
    restartForAutoUpdate,
    loadDeskVersion,
    checkDeskUpdates,
    checkDllUpdates,
    checkAllUpdates,
  };
}
