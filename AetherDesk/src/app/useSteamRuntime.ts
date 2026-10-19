import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { STEAM_RUNTIME_EVENT } from '../constants/library';
import { useVisiblePolling } from '../hooks/useVisiblePolling';

/**
 * Safety net for the push channel, mirroring the Library's revision
 * reconciliation (`useLibraryGames`): `steam://runtime-state` is the primary
 * signal, but a single missed push (throttled/suspended webview, subscribe
 * race) used to leave the Sidebar label stuck on "Restart Steam" until the
 * next Steam transition or a manual action. Re-reading the O(1) backend state
 * while the window is visible makes the label self-healing.
 */
export const STEAM_RECONCILE_INTERVAL_MS = 2_000;
const STEAM_RECONCILE_BACKOFF_MS = 15_000;

/** Tracks Steam process state and serializes Start/Restart button actions. */
export function useSteamRuntime() {
  const [running, setRunning] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);

  const runAction = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      const isRunning = running ?? (await invoke<boolean>('is_steam_running'));
      const message = isRunning ? await invoke<string>('restart_steam') : await invoke<string>('start_steam');
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

  // Primary channel: real-time transition push from the shared monitor
  // (core::steam_monitor) + one-shot hydration for the first paint. A missed
  // push is recovered by the reconcile tick below.
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    // Set when the effect cleans up before `listen` resolves: the late
    // `.then` must unsubscribe immediately instead of leaking the handler.
    let cancelled = false;
    void invoke<boolean>('is_steam_running')
      .then((isRunning) => {
        if (active) setRunning(isRunning);
      })
      .catch((error) => console.warn('is_steam_running failed:', error));
    void listen<boolean>(STEAM_RUNTIME_EVENT, (event) => {
      setRunning(event.payload);
    })
      .then((off) => {
        if (cancelled) {
          off();
        } else {
          unlisten = off;
        }
      })
      .catch((error) => console.warn('steam runtime subscription failed:', error));
    return () => {
      active = false;
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Safety net: every `intervalMs` while the document is visible (with a
  // backoff when it is hidden, plus an immediate catch-up on becoming visible
  // again) the O(1) backend state is read once more, so a lost push cannot
  // leave the label stale.
  const reconcile = useCallback(async () => {
    try {
      setRunning(await invoke<boolean>('is_steam_running'));
    } catch {
      // Keep the last known state: the backend monitor keeps polling and the
      // next reconcile tick reads it again.
    }
  }, []);

  useVisiblePolling(reconcile, {
    intervalMs: STEAM_RECONCILE_INTERVAL_MS,
    backoffMs: STEAM_RECONCILE_BACKOFF_MS,
    // Hydration is owned by the effect above: the first reconcile waits for
    // the interval instead of duplicating the mount read.
    immediate: false,
  });

  return { running, busy, runAction };
}
