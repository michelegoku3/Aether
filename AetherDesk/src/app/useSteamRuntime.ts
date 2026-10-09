import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { STEAM_RUNTIME_EVENT } from '../constants/library';

/** Tracks Steam process state and serializes Start/Restart button actions. */
export function useSteamRuntime() {
  const [running, setRunning] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);

  const runAction = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      const isRunning = running ?? await invoke<boolean>('is_steam_running');
      const message = isRunning
        ? await invoke<string>('restart_steam')
        : await invoke<string>('start_steam');
      console.log('Steam action done:', message);
      setRunning(await invoke<boolean>('is_steam_running'));
    } catch (error) {
      console.error('Failed to start/restart Steam:', error);
      try {
        setRunning(await invoke<boolean>('is_steam_running'));
      } catch {
        // The backend monitor retries on its next tick.
      }
    } finally {
      setBusy(false);
    }
  }, [busy, running]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    void invoke<boolean>('is_steam_running')
      .then((isRunning) => { if (active) setRunning(isRunning); })
      .catch((error) => console.warn('is_steam_running failed:', error));
    void listen<boolean>(STEAM_RUNTIME_EVENT, (event) => {
      setRunning(event.payload);
    }).then((off) => { unlisten = off; });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  return { running, busy, runAction };
}
