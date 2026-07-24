import { useState, useEffect } from 'react';
import { Sidebar, TabType } from './components/Sidebar';
import { MainContent } from './components/MainContent';
import { invoke } from '@tauri-apps/api/core';

export default function App() {
  // Setup state to manage the active view, defaulting to 'home'
  const [activeTab, setActiveTab] = useState<TabType>('home');

  // Global state to track AetherDLL update availability from live GitHub Release API
  const [dllUpdateAvailable, setDllUpdateAvailable] = useState(false);
  // Global state to track native AetherDesk update availability from desk-* GitHub tags
  const [deskUpdateAvailable, setDeskUpdateAvailable] = useState(false);

  // Method to check for component updates globally (runs on mount and after operations)
  const checkUpdates = async () => {
    try {
      const deskInfo: any = await invoke('check_aether_desk_update');
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
        setDllUpdateAvailable(updateInfo.update_available);
      }
    } catch (err) {
      console.error("AetherDLL update check failed:", err);
    }
  };

  // Run update checks on startup and then only occasionally.
  // GitHub public API is limited to 60 unauthenticated requests/hour per IP: polling every
  // 45 seconds can quickly cause 403 Forbidden/rate-limit errors, especially because Desk
  // and DLL are checked separately.
  useEffect(() => {
    checkUpdates();
    const interval = setInterval(checkUpdates, 30 * 60 * 1000);
    return () => clearInterval(interval);
  }, []);

  // Professional decoupled action handler to kill and restart Steam via Rust
  const handleRestartSteam = async () => {
    try {
      await invoke('restart_steam');
      alert('Steam has been terminated and is restarting asynchronously in the background.');
    } catch (err: any) {
      alert(`Failed to restart Steam: ${err}`);
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
      />
    </div>
  );
}
