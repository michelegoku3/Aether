import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { Sidebar, TabType } from './layout/Sidebar';
import { MainContent } from './layout/MainContent';
import { DllStatusInfo } from './types/ui';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useCustomCss } from './hooks/useCustomCss';
import { usePersonalWallpaper } from './hooks/usePersonalWallpaper';
import { STEAM_RUNTIME_EVENT } from './constants/library';
import { LibraryGamesProvider } from './hooks/useLibraryGames';
import { AppSettings, getSettings, hasValidSteamPath } from './hooks/useSettings';
import { SteamPathWarningModal } from './modals/SteamPathWarningModal';
import { UnsavedChangesModal } from './modals/UnsavedChangesModal';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { SettingsGuard } from './views/SettingsView';
import type {
  MainContentAppearance,
  MainContentDll,
  MainContentSettings,
  MainContentUpdates,
} from './layout/MainContent';

export default function App() {
  // Setup state to manage the active view, defaulting to 'home'
  const [activeTab, setActiveTab] = useState<TabType>('home');
  // No-Steam-path warning (OST-style modal): shown on startup, when leaving
  // Settings, or when saving without a valid path — until one is configured.
  const [showSteamPathWarning, setShowSteamPathWarning] = useState(false);
  // Unsaved-changes guard: SettingsView publishes isDirty/save/discard here;
  // tab switches and window close consult it before proceeding.
  const settingsGuardRef = useRef<SettingsGuard | null>(null);
  const [showUnsavedModal, setShowUnsavedModal] = useState(false);
  const [unsavedBusy, setUnsavedBusy] = useState(false);
  const pendingActionRef = useRef<{ type: 'tab'; tab: TabType } | { type: 'close' } | null>(null);

  /** Shows the Steam-path warning unless the stored path is a valid
   *  installation. Shared by the startup check and the tab-change guard.
   *  Deps vuote: legge solo lo stato persistito, quindi l'identità è stabile. */
  const warnIfSteamPathMissing = useCallback(async () => {
    if (!(await hasValidSteamPath())) {
      setShowSteamPathWarning(true);
    }
  }, []);

  /** Leaving Settings without a valid stored Steam path pops the warning.
   *  Navigation itself is never blocked — the modal offers "Go to Settings".
   *  Unsaved form edits take precedence: they pop the unsaved-changes modal
   *  instead, and the switch only completes after Save / Don't Save. */
  const handleTabChange = useCallback((tab: TabType) => {
    if (showUnsavedModal) return; // decide first — the modal owns the pending action
    if (activeTab === 'settings' && tab !== 'settings') {
      if (settingsGuardRef.current?.isDirty()) {
        pendingActionRef.current = { type: 'tab', tab };
        setShowUnsavedModal(true);
        return;
      }
      void warnIfSteamPathMissing();
    }
    setActiveTab(tab);
  }, [activeTab, showUnsavedModal, warnIfSteamPathMissing]);

  /** Completes a guard-approved pending action (tab switch or window close). */
  const completePendingAction = useCallback(async (pending: { type: 'tab'; tab: TabType } | { type: 'close' }) => {
    if (pending.type === 'tab') {
      setActiveTab(pending.tab);
      void warnIfSteamPathMissing();
    } else {
      // Default close was prevented: destroy via backend (custom commands
      // need no capability grant, unlike core window APIs from the frontend).
      try {
        await invoke('force_close_window');
      } catch (err) {
        console.error('Failed to close window:', err);
      }
    }
  }, [warnIfSteamPathMissing]);

  const handleUnsavedSave = useCallback(async () => {
    if (unsavedBusy) return;
    setUnsavedBusy(true);
    try {
      const saved = await settingsGuardRef.current?.save();
      if (!saved) return; // toast/steam modal already shown; keep modal open to retry
      const pending = pendingActionRef.current;
      pendingActionRef.current = null;
      setShowUnsavedModal(false);
      if (pending) await completePendingAction(pending);
    } finally {
      setUnsavedBusy(false);
    }
  }, [unsavedBusy, completePendingAction]);

  const handleUnsavedDiscard = useCallback(() => {
    if (unsavedBusy) return;
    const pending = pendingActionRef.current;
    pendingActionRef.current = null;
    setShowUnsavedModal(false);
    settingsGuardRef.current?.discard();
    if (pending) void completePendingAction(pending);
  }, [unsavedBusy, completePendingAction]);

  const handleUnsavedCancel = useCallback(() => {
    if (unsavedBusy) return;
    pendingActionRef.current = null;
    setShowUnsavedModal(false);
  }, [unsavedBusy]);

  // Global state to track AetherDLL update availability from GitHub release tags
  const [dllUpdateAvailable, setDllUpdateAvailable] = useState(false);
  // Global state to track native AetherDesk update availability from desk-* GitHub tags
  const [deskVersion, setDeskVersion] = useState('…');
  const [deskUpdateAvailable, setDeskUpdateAvailable] = useState(false);
  // Whether the currently available desk/dll update is a *test* build (red dot).
  const [deskUpdateIsTest, setDeskUpdateIsTest] = useState(false);
  const [dllUpdateIsTest, setDllUpdateIsTest] = useState(false);
  const [useAlternativeGameCards, setUseAlternativeGameCards] = useState(false);

  // DLL installation status (checked only at startup and after install/uninstall)
  const [dllStatus, setDllStatus] = useState<DllStatusInfo>({
    isInstalled: false,
    installedVersion: 'N/A',
    isSteamBlocked: false
  });

  // Hubcap API usage limits
  const [hubcapUsage, setHubcapUsage] = useState({ usage: 0, limit: 1500, hasKey: false });

  // Settings revision: incremented after Settings saves/resets so always-mounted
  // views (Store) can reload data that depends on settings without restarting.
  const [settingsRevision, setSettingsRevision] = useState(0);
  // Avoid a Store preload against default/incomplete settings while the initial
  // settings hydration is still in flight.
  const [settingsReady, setSettingsReady] = useState(false);

  // Appearance toggles — single source of truth for the whole app.
  const [customCssEnabled, setCustomCssEnabled] = useState(false);
  const [personalWallpaperEnabled, setPersonalWallpaperEnabled] = useState(false);
  // null = prima lettura ancora in corso; boolean = stato noto del monitor.
  const [steamRunning, setSteamRunning] = useState<boolean | null>(null);
  const [personalWallpaperOpacity, setPersonalWallpaperOpacity] = useState(20);
  const [alternativeCardsOpacity, setAlternativeCardsOpacity] = useState(100);
  const [alternativeCardsFade, setAlternativeCardsFade] = useState(50);
  // Bumped whenever the wallpaper file selection changes so the hook re-reads
  // the data URI even when `enabled` stays true.
  const [wallpaperRevision, setWallpaperRevision] = useState(0);
  // Bumped whenever the theme changes (toggle, picker, settings save) so the
  // custom-CSS hook re-fetches and applies the theme in real time.
  const [themeRevision, setThemeRevision] = useState(0);
  /** Applies a settings snapshot to local React state. Centralising the mapping
   *  (instead of re-fetching inside every startup/refresh path) lets callers
   *  share a single `get_settings` result across many dependents, eliminating
   *  the startup N+1 IPC pattern reported by the audit. */
  const applySettingsSnapshot = useCallback((settings: AppSettings) => {
    setUseAlternativeGameCards(Boolean(settings.use_alternative_game_cards));
    setCustomCssEnabled(Boolean(settings.custom_css_enabled));
    setPersonalWallpaperEnabled(Boolean(settings.personal_wallpaper_enabled));
    setPersonalWallpaperOpacity(Math.max(0, Math.min(100, Number(settings.personal_wallpaper_opacity ?? 20))));
    setAlternativeCardsOpacity(Math.max(0, Math.min(100, Number(settings.alternative_cards_opacity ?? 100))));
    setAlternativeCardsFade(Math.max(0, Math.min(100, Number(settings.alternative_cards_fade ?? 50))));
  }, []);

  // Check DLL installation status (called at startup and after install/uninstall).
  // Accepts an optional preloaded settings snapshot so callers that already
  // fetched settings (e.g. startup) can skip the redundant IPC round-trip.
  // Dichiarata PRIMA di refreshCustomCss, che la usa come dependency: con
  // useCallback l'ordine di dichiarazione conta (TDZ), prima non contava.
  const checkDllStatus = useCallback(async (preloadedSettings?: AppSettings) => {
    try {
      const settings = preloadedSettings ?? await getSettings();

      if (settings.steam_path && settings.steam_path.trim() !== '') {
        const [isInstalled, isBlocked, updateInfo] = await Promise.all([
          invoke<boolean>('is_dll_installed'),
          invoke<boolean>('is_steam_blocked'),
          invoke<DllUpdateInfo>('check_aether_dll_update'),
        ]);

        setDllStatus({
          isInstalled,
          installedVersion: updateInfo.installed_version || 'N/A',
          isSteamBlocked: isBlocked
        });
      }
    } catch (err) {
      console.error("Failed to check DLL status:", err);
      setDllStatus({
        isInstalled: false,
        installedVersion: 'N/A',
        isSteamBlocked: false
      });
    }
  }, []);

  const refreshCustomCss = useCallback(async (preloadedSettings?: AppSettings) => {
    try {
      const settings = preloadedSettings ?? await getSettings();
      applySettingsSnapshot(settings);
      setSettingsRevision((value) => value + 1);
      setWallpaperRevision((value) => value + 1);
      setThemeRevision((value) => value + 1);
      // A settings save can change the Steam root: re-resolve DLL install
      // state too, so the Aether panel never shows a stale previous path.
      // Pass the snapshot we already have so checkDllStatus skips its own
      // redundant get_settings call.
      void checkDllStatus(settings);
    } catch {
      setUseAlternativeGameCards(false);
      setCustomCssEnabled(false);
      setPersonalWallpaperEnabled(false);
      setPersonalWallpaperOpacity(20);
      setAlternativeCardsOpacity(100);
      setAlternativeCardsFade(50);
    } finally {
      setSettingsReady(true);
    }
  }, [applySettingsSnapshot, checkDllStatus]);
  // Real-time theme toggling: called straight from the Settings switch (no
  // "Save Settings" needed). Also re-reads the theme file immediately.
  const changeCustomCss = useCallback((enabled: boolean) => {
    setCustomCssEnabled(enabled);
    setThemeRevision((value) => value + 1);
  }, []);
  const previewPersonalWallpaper = useCallback((enabled: boolean, opacity: number) => {
    setPersonalWallpaperEnabled(enabled);
    setPersonalWallpaperOpacity(Math.max(0, Math.min(100, opacity)));
  }, []);
  const previewAlternativeCards = useCallback((opacity: number, fade: number) => {
    setAlternativeCardsOpacity(Math.max(0, Math.min(100, opacity)));
    setAlternativeCardsFade(Math.max(0, Math.min(100, fade)));
  }, []);
  useCustomCss(customCssEnabled, themeRevision);
  usePersonalWallpaper(personalWallpaperEnabled, personalWallpaperOpacity, wallpaperRevision);

  interface DeskUpdateInfo { update_available?: boolean; is_test?: boolean; installed_version?: string; }
  interface DllUpdateInfo { update_available?: boolean; is_test?: boolean; installed_version?: string; }
  interface HubcapUsageStats { usage: number; limit: number; }

  const refreshHubcapUsage = useCallback(async (forcedKey?: string, preloadedSettings?: AppSettings) => {
    try {
      const key = forcedKey ?? (preloadedSettings ?? await getSettings()).hubcap_api_key;
      if (key && key.trim() !== '') {
        const stats = await invoke<HubcapUsageStats>('get_hubcap_usage', { apiKey: key });
        setHubcapUsage({ usage: stats.usage, limit: stats.limit, hasKey: true });
      } else {
        setHubcapUsage({ usage: 0, limit: 1500, hasKey: false });
      }
    } catch (err) {
      console.error("Failed to fetch Hubcap usage:", err);
      setHubcapUsage({ usage: 0, limit: 1500, hasKey: false });
    }
  }, []);

  /** Desk update check is independent of Steam/settings and can run in
   *  parallel with the DLL check (which needs a configured steam_path). */
  const checkDeskUpdates = useCallback(async () => {
    try {
      const deskInfo = await invoke<DeskUpdateInfo>('check_aether_desk_update');
      console.log('[AetherDesk update check]', deskInfo);
      setDeskUpdateAvailable(Boolean(deskInfo.update_available));
      setDeskUpdateIsTest(Boolean(deskInfo.is_test));
      if (deskInfo.installed_version) {
        setDeskVersion(deskInfo.installed_version);
      }
    } catch (err) {
      console.error("AetherDesk update check failed:", err);
      setDeskUpdateAvailable(false);
      setDeskUpdateIsTest(false);
    }
  }, []);

  const checkDllUpdates = useCallback(async (settings: AppSettings) => {
    try {
      // Gate lato client SOLO per non fare IPC + chiamate GitHub a vuoto quando
      // Steam non è configurato. Il percorso vero lo risolve il backend dalle
      // impostazioni: non viaggia più nel payload (docs/shared_contracts.md §8).
      if (settings.steam_path && settings.steam_path.trim() !== '') {
        const updateInfo = await invoke<DllUpdateInfo>('check_aether_dll_update');
        console.log('[AetherDLL update check]', updateInfo);
        setDllUpdateAvailable(Boolean(updateInfo.update_available));
        setDllUpdateIsTest(Boolean(updateInfo.is_test));
      } else {
        setDllUpdateAvailable(false);
        setDllUpdateIsTest(false);
      }
    } catch (err) {
      console.error("AetherDLL update check failed:", err);
      setDllUpdateAvailable(false);
      setDllUpdateIsTest(false);
    }
  }, []);

  /** Manual re-check of desk+dll update availability (triggered from the
   *  Aether panel "Refresh" button). Mirrors the parallel startup pattern so
   *  refreshing doesn't refetch settings twice either. */
  const checkAllUpdates = useCallback(async () => {
    const settings = await getSettings();
    await Promise.all([
      checkDeskUpdates(),
      checkDllUpdates(settings),
    ]);
  }, [checkDeskUpdates, checkDllUpdates]);

  // Startup hydration: previously App.tsx issued four `get_settings` calls
  // serially (refreshCustomCss + refreshHubcapUsage + checkUpdates.DLL +
  // checkDllStatus) plus two update checks. We now fetch settings ONCE and
  // fan-out the snapshot to every consumer via the preloadedSettings
  // parameter; independent calls (desk version, library cache warm-up) run
  // in parallel with Promise.all so the UI settles in one round-trip.
  useEffect(() => {
    const hydrate = async () => {
      // Kick off independent work first (no shared state with settings).
      const deskVersionPromise = invoke<string>('get_desk_version')
        .then((v) => setDeskVersion(v || 'N/A'))
        .catch(() => setDeskVersion('N/A'));
      const warmPromise = invoke<number>('warm_library_game_cache')
        .then(count => console.log(`[AetherDesk library cache warm-up] ${count} cached names available`))
        .catch(err => console.warn('Library cache warm-up failed:', err));

      // Single settings fetch shared by CSS/Hubcap/DLL dependents.
      const settingsPromise = getSettings();
      const [settings] = await Promise.all([settingsPromise, deskVersionPromise, warmPromise]);

      // Fan out the snapshot in parallel — none of these depend on each other.
      await Promise.all([
        refreshCustomCss(settings),
        refreshHubcapUsage(undefined, settings),
        checkDeskUpdates(),
        checkDllUpdates(settings),
        warnIfSteamPathMissing(),
      ]);
    };
    void hydrate();
  }, []);

  // Window-close guard: with unsaved Settings edits, prevent the default
  // close and prompt instead (Save/Discard complete via force_close_window).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    getCurrentWindow().onCloseRequested((event) => {
      if (settingsGuardRef.current?.isDirty()) {
        event.preventDefault();
        // Overwrites any tab-switch pending action: closing wins.
        pendingActionRef.current = { type: 'close' };
        setShowUnsavedModal(true);
      }
    }).then((off) => { unlisten = off; });
    return () => unlisten?.();
  }, []);

  // Steam action: Start (solo spawn, mai kill) vs Restart (kill + wait +
  // respawn) — i due comandi backend hanno semantica esplicita, qui si sceglie
  // in base allo stato del monitor condiviso. `steamBusy` blocca il doppio
  // click durante il restart (che include l'attesa dell'uscita del processo).
  const [steamBusy, setSteamBusy] = useState(false);
  const handleSteamAction = useCallback(async () => {
    if (steamBusy) return;
    setSteamBusy(true);
    try {
      const running = steamRunning ?? await invoke<boolean>('is_steam_running');
      const message = running
        ? await invoke<string>('restart_steam')
        : await invoke<string>('start_steam');
      console.log('Steam action done:', message);
      // Do NOT optimistically `setSteamRunning(true)` here: we let the
      // dedicated `steam://runtime-state` event (pushed by the backend
      // monitor when the transition actually completes) converge the label.
      // If the event is slow to arrive, fall back to an explicit read once.
      setSteamRunning(await invoke<boolean>('is_steam_running'));
    } catch (err: any) {
      console.error('Failed to start/restart Steam:', err);
      try {
        setSteamRunning(await invoke<boolean>('is_steam_running'));
      } catch { /* il monitor ci riprova al prossimo tick */ }
    } finally {
      setSteamBusy(false);
    }
  }, [steamBusy, steamRunning]);

  // Live Steam presence: initial O(1) read from the backend monitor, then
  // push events on every state transition (no per-render process scans).
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    invoke<boolean>('is_steam_running')
      .then((running) => { if (active) setSteamRunning(running); })
      .catch((err) => console.warn('is_steam_running failed:', err));
    listen<boolean>(STEAM_RUNTIME_EVENT, (event) => {
      setSteamRunning(event.payload);
    }).then((off) => { unlisten = off; });
    return () => { active = false; unlisten?.(); };
  }, []);

  // ---- Gruppi di props per MainContent -----------------------------------
  // `MainContent` è `memo`: confronta le props con `===`. Costruire i quattro
  // gruppi inline nel JSX (come prima) produrrebbe oggetti nuovi a ogni render
  // di App, quindi il memo non scatterebbe mai e il raggruppamento resterebbe
  // puramente estetico. Con `useMemo` l'identità cambia solo quando cambia un
  // valore che il gruppo espone davvero.
  const updatesGroup = useMemo<MainContentUpdates>(() => ({
    dllAvailable: dllUpdateAvailable,
    deskAvailable: deskUpdateAvailable,
    deskVersion,
    dllIsTest: dllUpdateIsTest,
    deskIsTest: deskUpdateIsTest,
    onComplete: checkAllUpdates,
  }), [
    dllUpdateAvailable,
    deskUpdateAvailable,
    deskVersion,
    dllUpdateIsTest,
    deskUpdateIsTest,
    checkAllUpdates,
  ]);

  const appearanceGroup = useMemo<MainContentAppearance>(() => ({
    useAlternativeGameCards,
    alternativeCardsOpacity,
    alternativeCardsFade,
    onRefreshCustomCss: refreshCustomCss,
    onCustomCssChange: changeCustomCss,
    onPreviewPersonalWallpaper: previewPersonalWallpaper,
    onPreviewAlternativeCards: previewAlternativeCards,
  }), [
    useAlternativeGameCards,
    alternativeCardsOpacity,
    alternativeCardsFade,
    refreshCustomCss,
    changeCustomCss,
    previewPersonalWallpaper,
    previewAlternativeCards,
  ]);

  // Callback dedicata: nel JSX era una arrow inline, cioè una prop nuova a ogni
  // render anche con il gruppo in useMemo.
  const showSteamPathWarningModal = useCallback(() => setShowSteamPathWarning(true), []);

  const settingsGroup = useMemo<MainContentSettings>(() => ({
    ready: settingsReady,
    revision: settingsRevision,
    guardRef: settingsGuardRef,
    onMissingSteamPath: showSteamPathWarningModal,
    hubcapUsage,
    onRefreshUsage: refreshHubcapUsage,
  }), [
    settingsReady,
    settingsRevision,
    hubcapUsage,
    refreshHubcapUsage,
    showSteamPathWarningModal,
  ]);

  const dllGroup = useMemo<MainContentDll>(() => ({
    status: dllStatus,
    onChange: checkDllStatus,
  }), [dllStatus, checkDllStatus]);

  return (
    <LibraryGamesProvider>
      <div className="app-container">
        {/* Modular Sidebar component with action hooks and global update badge */}
        <Sidebar
          activeTab={activeTab}
          onTabChange={handleTabChange}
          onSteamAction={handleSteamAction}
          steamRunning={steamRunning}
          steamBusy={steamBusy}
          dllUpdateAvailable={dllUpdateAvailable || deskUpdateAvailable}
          updateIsTest={dllUpdateIsTest || deskUpdateIsTest}
        />

        {/* Modular Main Content display area */}
        <MainContent
          activeTab={activeTab}
          updates={updatesGroup}
          appearance={appearanceGroup}
          settings={settingsGroup}
          dll={dllGroup}
        />

        {showUnsavedModal && (
          <UnsavedChangesModal
            busy={unsavedBusy}
            onSave={() => void handleUnsavedSave()}
            onDiscard={handleUnsavedDiscard}
            onCancel={handleUnsavedCancel}
          />
        )}

        {showSteamPathWarning && (
          <SteamPathWarningModal
            onGoToSettings={() => {
              setShowSteamPathWarning(false);
              setActiveTab('settings');
            }}
            onClose={() => setShowSteamPathWarning(false)}
          />
        )}
      </div>
    </LibraryGamesProvider>
  );
}
