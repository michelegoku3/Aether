import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { emptyStatus, StatusMessage } from '../types/ui';
import { LUA_LIBRARY_EVENT } from '../constants/library';

export interface InstalledGame {
  id: number;
  name: string;
  appId: string;
  installDir: string;
  libraryPath: string;
  gamePath: string;
  installed: boolean;
  imageUrl?: string;
  heroImageUrl?: string;
}

/**
 * Library scan state, shared by every consumer (Home, Library, modals).
 *
 * Subscribes to the backend `library://lua-changed` event (emitted by
 * core/library_events.rs): when a .lua install operation succeeds — store
 * downloads from hubcap/luatool/ryuu/moed, local install/bulk import,
 * in-app removal — a background rescan runs so the library is current
 * without a manual Refresh. Re-entrancy guarded: if a rescan is already
 * running, the change is coalesced into one follow-up.
 */
export const useLibraryGames = () => {
  const [games, setGames] = useState<InstalledGame[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [status, setStatus] = useState<StatusMessage>(emptyStatus());

  const loadingRef = useRef(false);
  const pendingRef = useRef(false);

  const loadInstalledGames = async () => {
    if (loadingRef.current) {
      // Un refresh è già in volo: il watcher può aver segnalato altri cambi,
      // coalesciamo in un follow-up singolo alla fine di questo.
      pendingRef.current = true;
      return;
    }
    loadingRef.current = true;
    pendingRef.current = false;
    setIsLoading(true);
    setStatus(emptyStatus());

    try {
      const result: InstalledGame[] = await invoke('get_installed_library_games');
      setGames(result || []);
    } catch (err: any) {
      setStatus({ text: `Failed to scan Steam library: ${err}`, type: 'error' });
    } finally {
      loadingRef.current = false;
      setIsLoading(false);
      if (pendingRef.current) {
        pendingRef.current = false;
        void loadInstalledGames();
      }
    }
  };

  useEffect(() => {
    loadInstalledGames();
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    listen(LUA_LIBRARY_EVENT, () => {
      void loadInstalledGames();
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    }).catch(() => {
      // Event bridge unavailable (shouldn't happen inside Tauri): the manual
      // Refresh button remains the fallback.
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
    // loadInstalledGames is stable across renders (refs + setState only).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return {
    games,
    isLoading,
    status,
    setStatus,
    loadInstalledGames,
  };
};
