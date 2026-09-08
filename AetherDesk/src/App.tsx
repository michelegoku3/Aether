import { useState, useEffect, useRef } from 'react';
import { Sidebar, TabType } from './layout/Sidebar';
import { MainContent } from './layout/MainContent';
import { DllStatusInfo } from './types/ui';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useCustomCss } from './hooks/useCustomCss';
import { usePersonalWallpaper } from './hooks/usePersonalWallpaper';
import { STEAM_RUNTIME_EVENT } from './constants/library';
import { LibraryGamesProvider } from './hooks/useLibraryGames';
import { hasValidSteamPath } from './hooks/useSettings';
import { SteamPathWarningModal } from './modals/SteamPathWarningModal';
import { UnsavedChangesModal } from './modals/UnsavedChangesModal';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { SettingsGuard } from './views/SettingsView';

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
   *  installation. Shared by the startup check and the tab-change guard. */
  const warnIfSteamPathMissing = async () => {
    if (!(await hasValidSteamPath())) {
      setShowSteamPathWarning(true);
    }
  };

  /** Leaving Settings without a valid stored Steam path pops the warning.
   *  Navigation itself is never blocked — the modal offers "Go to Settings".
   *  Unsaved form edits take precedence: they pop the unsaved-changes modal
   *  instead, and the switch only completes after Save / Don't Save. */
  const handleTabChange = (tab: TabType) => {
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
  };

  /** Completes a guard-approved pending action (tab switch or window close). */
  const completePendingAction = async (pending: { type: 'tab'; tab: TabType } | { type: 'close' }) => {
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
  };

  const handleUnsavedSave = async () => {
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
  };

  const handleUnsavedDiscard = () => {
    if (unsavedBusy) return;
    const pending = pendingActionRef.current;
    pendingActionRef.current = null;
    setShowUnsavedModal(false);
    settingsGuardRef.current?.discard();
    if (pending) void completePendingAction(pending);
  };

  const handleUnsavedCancel = () => {
    if (unsavedBusy) return;
    pendingActionRef.current = null;
    setShowUnsavedModal(false);
  };

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
  const [hubcapUsage, setHubcapUsage] = useState({ usage: 0, limit: 25, hasKey: false });

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
  const refreshCustomCss = async () => {
    try {
      const settings: any = await invoke('get_settings');
      setUseAlternativeGameCards(Boolean(settings.use_alternative_game_cards));
      setCustomCssEnabled(Boolean(settings.custom_css_enabled));
      setPersonalWallpaperEnabled(Boolean(settings.personal_wallpaper_enabled));
      setPersonalWallpaperOpacity(Math.max(0, Math.min(100, Number(settings.personal_wallpaper_opacity ?? 20))));
      setAlternativeCardsOpacity(Math.max(0, Math.min(100, Number(settings.alternative_cards_opacity ?? 100))));
      setAlternativeCardsFade(Math.max(0, Math.min(100, Number(settings.alternative_cards_fade ?? 50))));
      setSettingsRevision((value) => value + 1);
      setWallpaperRevision((value) => value + 1);
      setThemeRevision((value) => value + 1);
      // A settings save can change the Steam root: re-resolve DLL install
      // state too, so the Aether panel never shows a stale previous path.
      void checkDllStatus();
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
  };
  // Real-time theme toggling: called straight from the Settings switch (no
  // "Save Settings" needed). Also re-reads the theme file immediately.
  const changeCustomCss = (enabled: boolean) => {
    setCustomCssEnabled(enabled);
    setThemeRevision((value) => value + 1);
  };
  const previewPersonalWallpaper = (enabled: boolean, opacity: number) => {
    setPersonalWallpaperEnabled(enabled);
    setPersonalWallpaperOpacity(Math.max(0, Math.min(100, opacity)));
  };
  const previewAlternativeCards = (opacity: number, fade: number) => {
    setAlternativeCardsOpacity(Math.max(0, Math.min(100, opacity)));
    setAlternativeCardsFade(Math.max(0, Math.min(100, fade)));
  };
  useCustomCss(customCssEnabled, themeRevision);
  usePersonalWallpaper(personalWallpaperEnabled, personalWallpaperOpacity, wallpaperRevision);

  const refreshHubcapUsage = async (forcedKey?: string) => {
    try {
      let key = forcedKey;
      if (key === undefined) {
        const settings: any = await invoke('get_settings');
        key = settings.hubcap_api_key;
      }
      if (key && key.trim() !== '') {
        const stats: any = await invoke('get_hubcap_usage', { apiKey: key });
        setHubcapUsage({ usage: stats.usage, limit: stats.limit, hasKey: true });
      } else {
        setHubcapUsage({ usage: 0, limit: 25, hasKey: false });
      }
    } catch (err) {
      console.error("Failed to fetch Hubcap usage:", err);
      setHubcapUsage({ usage: 0, limit: 25, hasKey: false });
    }
  };

  // Method to check for component updates globally (runs on mount and after operations)
  const checkUpdates = async () => {
    try {
      const deskInfo: any = await invoke('check_aether_desk_update');
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

    try {
      const settings: any = await invoke('get_settings');
      const steamPath = settings.steam_path;
      if (steamPath && steamPath.trim() !== '') {
        const updateInfo: any = await invoke('check_aether_dll_update', { steamPath });
        console.log('[AetherDLL update check]', updateInfo);
        setDllUpdateAvailable(updateInfo.update_available);
        setDllUpdateIsTest(Boolean(updateInfo.is_test));
      }
    } catch (err) {
      console.error("AetherDLL update check failed:", err);
      setDllUpdateAvailable(false);
      setDllUpdateIsTest(false);
    }
  };

  // Check DLL installation status (called at startup and after install/uninstall)
  const checkDllStatus = async () => {
    try {
      const settings: any = await invoke('get_settings');
      const steamPath = settings.steam_path;

      if (steamPath && steamPath.trim() !== '') {
        const isInstalled: any = await invoke('is_dll_installed', { steamPath });
        const isBlocked: any = await invoke('is_steam_blocked', { steamPath });
        const updateInfo: any = await invoke('check_aether_dll_update', { steamPath });

        setDllStatus({
          isInstalled,
          installedVersion: updateInfo.installed_version || 'N/A',
          isSteamBlocked: isBlocked
        });
      }
    } catch (err) {
      console.error("Failed to check DLL status:", err);
      // Unreachable Steam (or any IPC failure) means "unknown", which the UI
      // renders as not-installed — never leave a stale previous state.
      setDllStatus({
        isInstalled: false,
        installedVersion: 'N/A',
        isSteamBlocked: false
      });
    }
  };

  // Warm the Library metadata cache as soon as the app starts.
  // This is fire-and-forget: Library rendering must never wait for Steam network calls.
  useEffect(() => {
    invoke('warm_library_game_cache')
      .then(count => console.log(`[AetherDesk library cache warm-up] ${count} cached names available`))
      .catch(err => console.warn('Library cache warm-up failed:', err));
  }, []);

  // Update check runs once at startup. Close and reopen Aether to check again.
  useEffect(() => {
    // Resolve the desk version instantly (local IPC, no network) so the Aether
    // panel shows the correct value from its first render — the GitHub-backed
    // check below refreshes it later without any flicker.
    invoke<string>('get_desk_version')
      .then((v) => setDeskVersion(v || 'N/A'))
      .catch(() => setDeskVersion('N/A'));
    checkUpdates();
    refreshHubcapUsage();
    // refreshCustomCss also refreshes DLL status (single call, no duplicate).
    refreshCustomCss();
    // No valid stored Steam path → OST-style warning (reappears every
    // startup until a valid path is configured).
    void warnIfSteamPathMissing();
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
  const handleSteamAction = async () => {
    if (steamBusy) return;
    setSteamBusy(true);
    try {
      const running = steamRunning ?? await invoke<boolean>('is_steam_running');
      const message = running
        ? await invoke<string>('restart_steam')
        : await invoke<string>('start_steam');
      console.log('Steam action done:', message);
      // I comandi aggiornano subito lo stato del monitor (mark) e l'evento
      // `steam://runtime-state` segue; qui forziamo la convergenza
      // dell'etichetta (l'azione termina sempre con Steam avviato).
      setSteamRunning(true);
    } catch (err: any) {
      console.error('Failed to start/restart Steam:', err);
      try {
        setSteamRunning(await invoke<boolean>('is_steam_running'));
      } catch { /* il monitor ci riprova al prossimo tick */ }
    } finally {
      setSteamBusy(false);
    }
  };

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
          dllUpdateAvailable={dllUpdateAvailable}
          deskUpdateAvailable={deskUpdateAvailable}
          deskVersion={deskVersion}
          dllUpdateIsTest={dllUpdateIsTest}
          deskUpdateIsTest={deskUpdateIsTest}
          onUpdateComplete={checkUpdates}
          hubcapUsage={hubcapUsage}
          onRefreshUsage={refreshHubcapUsage}
          dllStatus={dllStatus}
          onDllStatusChange={checkDllStatus}
          onRefreshCustomCss={refreshCustomCss}
          onCustomCssChange={changeCustomCss}
          onPreviewPersonalWallpaper={previewPersonalWallpaper}
          onPreviewAlternativeCards={previewAlternativeCards}
          settingsRevision={settingsRevision}
          settingsReady={settingsReady}
          useAlternativeGameCards={useAlternativeGameCards}
          alternativeCardsOpacity={alternativeCardsOpacity}
          alternativeCardsFade={alternativeCardsFade}
          onMissingSteamPath={() => setShowSteamPathWarning(true)}
          settingsGuardRef={settingsGuardRef}
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
