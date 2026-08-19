import { useState, useEffect } from 'react';
import { Sidebar, TabType } from './layout/Sidebar';
import { MainContent } from './layout/MainContent';
import { DllStatusInfo } from './types/ui';
import { invoke } from '@tauri-apps/api/core';
import { useCustomCss } from './hooks/useCustomCss';
import { usePersonalWallpaper } from './hooks/usePersonalWallpaper';

export default function App() {
  // Setup state to manage the active view, defaulting to 'home'
  const [activeTab, setActiveTab] = useState<TabType>('home');

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

  // Appearance toggles — single source of truth for the whole app.
  const [customCssEnabled, setCustomCssEnabled] = useState(false);
  const [personalWallpaperEnabled, setPersonalWallpaperEnabled] = useState(false);
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
    } catch {
      setUseAlternativeGameCards(false);
      setCustomCssEnabled(false);
      setPersonalWallpaperEnabled(false);
      setPersonalWallpaperOpacity(20);
      setAlternativeCardsOpacity(100);
      setAlternativeCardsFade(50);
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
    checkDllStatus();
    refreshHubcapUsage();
    refreshCustomCss();
  }, []);

  // Professional decoupled action handler to kill and restart Steam via Rust.
  // Keep this silent: the sidebar button is an immediate utility action and should not
  // interrupt the user with browser-level alerts.
  const handleRestartSteam = async () => {
    try {
      await invoke('restart_steam');
      console.log('Steam restart requested successfully.');
    } catch (err: any) {
      console.error('Failed to restart Steam:', err);
    }
  };

  return (
    <div className="app-container">
      {/* Modular Sidebar component with action hooks and global update badge */}
      <Sidebar 
        activeTab={activeTab} 
        onTabChange={setActiveTab} 
        onRestartSteam={handleRestartSteam} 
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
        useAlternativeGameCards={useAlternativeGameCards}
        alternativeCardsOpacity={alternativeCardsOpacity}
        alternativeCardsFade={alternativeCardsFade}
      />
    </div>
  );
}
