import { useState, useEffect } from 'react';
import { Sidebar, TabType } from './components/Sidebar';
import { MainContent } from './components/MainContent';
import { invoke } from '@tauri-apps/api/core';

export default function App() {
  // Setup state to manage the active view, defaulting to 'home'
  const [activeTab, setActiveTab] = useState<TabType>('home');

  // Global state to track AetherDLL update availability from live GitHub Release API
  const [dllUpdateAvailable, setDllUpdateAvailable] = useState(false);

  // Method to check for DLL updates globally (runs on mount and after operations)
  const checkDllUpdates = async () => {
    try {
      const settings: any = await invoke('get_settings');
      const steamPath = settings.steam_path;
      if (steamPath && steamPath.trim() !== '') {
        const updateInfo: any = await invoke('check_aether_dll_update', { steamPath });
        setDllUpdateAvailable(updateInfo.update_available);
      }
    } catch (err) {
      console.error("Global update check failed:", err);
    }
  };

  // Run update checks on startup and poll every 45 seconds for a reactive experience!
  useEffect(() => {
    checkDllUpdates();
    const interval = setInterval(checkDllUpdates, 45000);
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
        dllUpdateAvailable={dllUpdateAvailable}
      />

      {/* Modular Main Content display area */}
      <MainContent 
        activeTab={activeTab} 
        dllUpdateAvailable={dllUpdateAvailable} 
        onUpdateComplete={checkDllUpdates}
      />
    </div>
  );
}
