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

  // Global state to track AetherDLL update availability from live GitHub Release API
  const [dllUpdateAvailable, setDllUpdateAvailable] = useState(false);
  // Global state to track native AetherDesk update availability from desk-* GitHub tags
  const [deskUpdateAvailable, setDeskUpdateAvailable] = useState(false);

  // DLL installation status (checked only at startup and after install/uninstall)
  const [dllStatus, setDllStatus] = useState<DllStatusInfo>({
    isInstalled: false,
    installedVersion: 'N/A',
    isSteamBlocked: false
  });

  // Hubcap API usage limits
  const [hubcapUsage, setHubcapUsage] = useState({ usage: 0, limit: 25, hasKey: false });

  // Appearance toggles — single source of truth for the whole app.
  const [customCssEnabled, setCustomCssEnabled] = useState(false);
  const [personalWallpaperEnabled, setPersonalWallpaperEnabled] = useState(false);
  const [personalWallpaperOpacity, setPersonalWallpaperOpacity] = useState(35);
  const refreshCustomCss = async () => {
    try {
      const settings: any = await invoke('get_settings');
      setCustomCssEnabled(Boolean(settings.custom_css_enabled));
      setPersonalWallpaperEnabled(Boolean(settings.personal_wallpaper_enabled));
      setPersonalWallpaperOpacity(Math.max(0, Math.min(100, Number(settings.personal_wallpaper_opacity ?? 35))));
    } catch {
      setCustomCssEnabled(false);
      setPersonalWallpaperEnabled(false);
      setPersonalWallpaperOpacity(35);
    }
  };
  const previewPersonalWallpaper = (enabled: boolean, opacity: number) => {
    setPersonalWallpaperEnabled(enabled);
    setPersonalWallpaperOpacity(Math.max(0, Math.min(100, opacity)));
  };
  useCustomCss(customCssEnabled);
  usePersonalWallpaper(personalWallpaperEnabled, personalWallpaperOpacity);

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
    } catch (err) {
      console.error("AetherDesk update check failed:", err);
      setDeskUpdateAvailable(false);
    }

    try {
      const settings: any = await invoke('get_settings');
      const steamPath = settings.steam_path;
      if (steamPath && steamPath.trim() !== '') {
        const updateInfo: any = await invoke('check_aether_dll_update', { steamPath });
        console.log('[AetherDLL update check]', updateInfo);
        setDllUpdateAvailable(updateInfo.update_available);
      }
    } catch (err) {
      console.error("AetherDLL update check failed:", err);
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

  // Run update checks on startup and then only occasionally.
  // GitHub public API is limited to 60 unauthenticated requests/hour per IP: polling every
  // 45 seconds can quickly cause 403 Forbidden/rate-limit errors, especially because Desk
  // and DLL are checked separately.
  useEffect(() => {
    checkUpdates();
    checkDllStatus();
    refreshHubcapUsage();
    refreshCustomCss();
    const interval = setInterval(checkUpdates, 30 * 60 * 1000);
    return () => clearInterval(interval);
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
      />

      {/* Modular Main Content display area */}
      <MainContent 
        activeTab={activeTab} 
        dllUpdateAvailable={dllUpdateAvailable}
        deskUpdateAvailable={deskUpdateAvailable}
        onUpdateComplete={checkUpdates}
        hubcapUsage={hubcapUsage}
        onRefreshUsage={refreshHubcapUsage}
        dllStatus={dllStatus}
        onDllStatusChange={checkDllStatus}
        onRefreshCustomCss={refreshCustomCss}
        onPreviewPersonalWallpaper={previewPersonalWallpaper}
      />
    </div>
  );
}
