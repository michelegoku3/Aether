import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { checkSteamPath, getSettings, type AppSettings, type SteamPathCheck } from '../../hooks/useSettings';
import type { SteamCheckStatus } from './types';

type StatusKind = 'info' | 'success' | 'error';

interface Params {
  steamPath: string;
  rawSettings: AppSettings;
  setSteamPath: (path: string) => void;
  acceptPersistedSettings: (settings: AppSettings) => void;
  refreshCustomCss: () => Promise<void>;
  showStatus: (text: string, kind: StatusKind) => void;
}

export function useSteamPath(params: Params) {
  const [checkStatus, setCheckStatus] = useState<SteamCheckStatus>({ state: 'idle', message: '' });
  const requestIdRef = useRef(0);

  useEffect(() => {
    const requestId = ++requestIdRef.current;
    setCheckStatus({ state: 'checking', message: 'Checking Steam path...' });
    const timer = window.setTimeout(() => void (async () => {
      let next: SteamCheckStatus;
      try {
        const result = await checkSteamPath(params.steamPath);
        next = result.valid ? { state: 'idle', message: '' }
          : { state: 'invalid', message: result.error || 'Invalid Steam path.' };
      } catch {
        next = { state: 'invalid', message: 'Could not validate the Steam path.' };
      }
      if (requestId === requestIdRef.current) setCheckStatus(next);
    })(), 400);
    return () => window.clearTimeout(timer);
  }, [params.steamPath]);

  const saveNow = useCallback(async (path: string, actionLabel: string) => {
    params.setSteamPath(path);
    let check: SteamPathCheck;
    try { check = await checkSteamPath(path); }
    catch { check = { valid: false, normalized: path, error: 'Could not validate the Steam path.' }; }
    if (!check.valid) {
      setCheckStatus({ state: 'invalid', message: check.error || 'Invalid Steam path.' });
      params.showStatus(`${actionLabel}, but it is not a valid Steam installation: ${check.error || path}`, 'error');
      return;
    }
    try {
      const base = await getSettings();
      const saved = await invoke<AppSettings>('save_settings', {
        settings: { ...base, steam_path: check.normalized }, base,
      });
      params.acceptPersistedSettings({ ...params.rawSettings, steam_path: saved.steam_path });
      params.setSteamPath(check.normalized);
      setCheckStatus({ state: 'idle', message: '' });
      params.showStatus(`${actionLabel}: ${check.normalized}`, 'success');
      await params.refreshCustomCss();
    } catch (error) {
      params.showStatus(`Failed to save Steam path: ${error}`, 'error');
    }
  }, [params]);

  const browse = useCallback(async () => {
    try {
      const path = await invoke<string | null>('pick_steam_folder');
      if (path) await saveNow(path, 'Steam folder selected');
    } catch (error) {
      params.showStatus(`Failed to open folder picker: ${error}`, 'error');
    }
  }, [params, saveNow]);

  const detect = useCallback(async () => {
    try {
      const path = await invoke<string | null>('detect_steam_path');
      if (!path) {
        params.showStatus('No Steam installation detected. Use Browse to select it manually.', 'error');
        return;
      }
      await saveNow(path, 'Steam auto-detected');
    } catch (error) {
      params.showStatus(`Steam auto-detection failed: ${error}`, 'error');
    }
  }, [params, saveNow]);

  return { checkStatus, browse, detect };
}
