import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { emptyStatus, type StatusMessage } from '../types/ui';
import { LUA_LIBRARY_EVENT, type LuaLibraryChange } from '../constants/library';

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

type RefreshOrigin = 'initial' | 'manual' | 'automatic';

// Native Tauri events are the primary invalidation transport. This inexpensive
// atomic-revision read is only a recovery channel for WebView event loss; it
// never polls Steam, `stplug-in`, or any other filesystem path.
const REVISION_RECONCILIATION_INTERVAL_MS = 1_500;

interface LibraryGamesContextValue {
  games: InstalledGame[];
  /** True only before the first completed Library scan. */
  isLoading: boolean;
  /** True while a later refresh preserves the last successful game list. */
  isRefreshing: boolean;
  status: StatusMessage;
  setStatus: (status: StatusMessage) => void;
  /** Same canonical backend scan used by the Library Refresh button. */
  loadInstalledGames: () => void;
}

const LibraryGamesContext = createContext<LibraryGamesContextValue | null>(null);

/**
 * Single source of truth for Lua library data across Library and Home.
 *
 * The provider subscribes before starting its initial scan, serializes all
 * refreshes, and coalesces any invalidations received while a scan is running.
 * The backend event is only an invalidation signal: every refresh calls
 * `get_installed_library_games`, exactly like the visible Refresh control.
 */
export const LibraryGamesProvider = ({ children }: { children: ReactNode }) => {
  const [games, setGames] = useState<InstalledGame[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [status, setStatus] = useState<StatusMessage>(emptyStatus());

  const mountedRef = useRef(true);
  const hasCompletedInitialScanRef = useRef(false);
  const loadingRef = useRef(false);
  const pendingRef = useRef<RefreshOrigin | null>(null);
  const requestRevisionRef = useRef(0);
  const lastObservedLibraryRevisionRef = useRef<number | null>(null);

  const scheduleRefresh = useCallback((origin: RefreshOrigin) => {
    if (loadingRef.current) {
      // Manual refresh has the strongest intent, but every origin resolves to
      // one canonical follow-up scan once the in-flight result is committed.
      if (origin === 'manual' || pendingRef.current === null) {
        pendingRef.current = origin;
      }
      return;
    }

    loadingRef.current = true;
    const requestRevision = ++requestRevisionRef.current;
    const isInitial = !hasCompletedInitialScanRef.current;
    if (mountedRef.current) {
      if (isInitial) {
        setIsLoading(true);
      } else {
        setIsRefreshing(true);
      }
      // Automatic filesystem/store invalidations must not clear status feedback
      // from an action the user just performed. Explicit Refresh starts clean.
      if (origin === 'manual') {
        setStatus(emptyStatus());
      }
    }

    void invoke<InstalledGame[]>('get_installed_library_games')
      .then((result) => {
        if (!mountedRef.current || requestRevision !== requestRevisionRef.current) return;
        setGames(result || []);
        hasCompletedInitialScanRef.current = true;
      })
      .catch((error: unknown) => {
        if (!mountedRef.current || requestRevision !== requestRevisionRef.current) return;
        const message = `Failed to scan Steam library: ${String(error)}`;
        if (origin === 'manual' || !hasCompletedInitialScanRef.current) {
          setStatus({ text: message, type: 'error' });
        } else {
          // Keep the last valid list fully usable after a background error.
          console.warn('[library] background refresh failed:', error);
        }
      })
      .finally(() => {
        loadingRef.current = false;
        if (mountedRef.current && requestRevision === requestRevisionRef.current) {
          setIsLoading(false);
          setIsRefreshing(false);
        }

        const pendingOrigin = pendingRef.current;
        pendingRef.current = null;
        if (pendingOrigin && mountedRef.current) {
          scheduleRefresh(pendingOrigin);
        }
      });
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let reconciliationTimer: number | undefined;
    let cancelled = false;

    const acceptRevision = (revision: number | undefined): boolean => {
      if (typeof revision !== 'number' || !Number.isFinite(revision)) {
        // A malformed legacy event is still an invalidation: do not make a
        // recoverable payload issue leave the visible Library stale.
        return true;
      }
      const previous = lastObservedLibraryRevisionRef.current;
      if (previous !== null && revision <= previous) return false;
      lastObservedLibraryRevisionRef.current = revision;
      return true;
    };

    const reconcileRevision = async () => {
      try {
        const revision = await invoke<number>('get_library_change_revision');
        if (cancelled) return;

        const previous = lastObservedLibraryRevisionRef.current;
        if (previous === null) {
          // The first baseline is covered by the initial complete scan below.
          lastObservedLibraryRevisionRef.current = revision;
          return;
        }
        if (revision > previous) {
          lastObservedLibraryRevisionRef.current = revision;
          console.warn('[library] recovered a missed change event at revision', revision);
          scheduleRefresh('automatic');
        }
      } catch (error) {
        // The pushed event path remains available if this optional safety net
        // cannot be called (for example during application shutdown).
        console.warn('[library] revision reconciliation unavailable:', error);
      }
    };

    const onVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        void reconcileRevision();
      }
    };

    const subscribeThenLoad = async () => {
      try {
        unlisten = await listen<LuaLibraryChange>(LUA_LIBRARY_EVENT, (event) => {
          if (acceptRevision(event.payload?.revision)) {
            scheduleRefresh('automatic');
          }
        });
      } catch (error) {
        // A manual Refresh and the revision safety net remain valid if the
        // event bridge is unavailable. Do not leave the Library stale.
        console.warn('[library] event bridge unavailable:', error);
      }

      if (cancelled) {
        unlisten?.();
        return;
      }

      // Subscribe first, then establish the revision baseline and scan. A
      // change before the baseline is included by the following full scan; a
      // change after it is either pushed or recovered by the next heartbeat.
      await reconcileRevision();
      if (cancelled) return;
      scheduleRefresh('initial');

      reconciliationTimer = window.setInterval(() => {
        if (document.visibilityState === 'visible') {
          void reconcileRevision();
        }
      }, REVISION_RECONCILIATION_INTERVAL_MS);
      document.addEventListener('visibilitychange', onVisibilityChange);
    };

    void subscribeThenLoad();
    return () => {
      cancelled = true;
      if (reconciliationTimer !== undefined) {
        window.clearInterval(reconciliationTimer);
      }
      document.removeEventListener('visibilitychange', onVisibilityChange);
      unlisten?.();
    };
  }, [scheduleRefresh]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const value: LibraryGamesContextValue = {
    games,
    isLoading,
    isRefreshing,
    status,
    setStatus,
    loadInstalledGames: () => scheduleRefresh('manual'),
  };

  return createElement(LibraryGamesContext.Provider, { value }, children);
};

export const useLibraryGames = (): LibraryGamesContextValue => {
  const context = useContext(LibraryGamesContext);
  if (!context) {
    throw new Error('useLibraryGames must be used inside LibraryGamesProvider');
  }
  return context;
};
