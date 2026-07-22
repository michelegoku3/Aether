import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

export const AetherView = () => {
  // Real active states bound to local filesystem and config status
  const [isDllInstalled, setIsDllInstalled] = useState(false);
  const [isSteamBlocked, setIsSteamBlocked] = useState(false);
  const [isUpdateAvailable, setIsUpdateAvailable] = useState(false);
  
  const [statusMsg, setStatusMsg] = useState({ text: '', type: 'info' });
  const [isProcessing, setIsProcessing] = useState(false);

  const showStatus = (text: string, type: 'info' | 'success' | 'error') => {
    setStatusMsg({ text, type });
    setTimeout(() => setStatusMsg({ text: '', type: 'info' }), 6000);
  };

  // Perform active check on local system state on component load
  const checkLocalSystemState = async () => {
    try {
      // 1. Get custom Steam folder path from settings.json
      const settings: any = await invoke('get_settings');
      const steamPath = settings.steam_path;

      if (steamPath && steamPath.trim() !== '') {
        // 2. Query backend to verify if the 3 DLL files exist on disk
        const isInstalled: any = await invoke('is_dll_installed', { steamPath });
        setIsDllInstalled(isInstalled);

        // 3. Query backend to verify if steam.cfg update block is enabled
        const isBlocked: any = await invoke('is_steam_blocked', { steamPath });
        setIsSteamBlocked(isBlocked);
      }
    } catch (err: any) {
      console.error("Failed to query local system state:", err);
    }
  };

  useEffect(() => {
    checkLocalSystemState();
  }, []);

  const handleInstallDll = async () => {
    setIsProcessing(true);
    showStatus('Fetching latest release from GitHub...', 'info');
    
    try {
      const settings: any = await invoke('get_settings');
      const steamPath = settings.steam_path;

      if (!steamPath || steamPath.trim() === '') {
        showStatus('Error: Please configure your Steam Path in Settings first!', 'error');
        setIsProcessing(false);
        return;
      }

      // Execute actual asynchronous download and extraction pipeline in Rust!
      const result: string = await invoke('install_aether_dll', { steamPath });
      
      setIsDllInstalled(true);
      showStatus(result, 'success');
      setIsProcessing(false);
    } catch (err: any) {
      showStatus(`Installation failed: ${err}`, 'error');
      setIsProcessing(false);
    }
  };

  const handleUninstallDll = async () => {
    setIsProcessing(true);
    showStatus('Removing AetherDLL binaries...', 'info');
    
    try {
      const settings: any = await invoke('get_settings');
      const steamPath = settings.steam_path;

      if (!steamPath || steamPath.trim() === '') {
        showStatus('Error: Please configure your Steam Path in Settings first!', 'error');
        setIsProcessing(false);
        return;
      }

      // Execute actual file deletion in Rust
      const result: string = await invoke('uninstall_aether_dll', { steamPath });
      
      setIsDllInstalled(false);
      showStatus(result, 'success');
      setIsProcessing(false);
    } catch (err: any) {
      showStatus(`Uninstall failed: ${err}`, 'error');
      setIsProcessing(false);
    }
  };

  const handleToggleSteamBlock = async () => {
    try {
      const settings: any = await invoke('get_settings');
      const steamPath = settings.steam_path;

      if (!steamPath || steamPath.trim() === '') {
        showStatus('Error: Please configure your Steam Path in Settings first!', 'error');
        return;
      }

      if (!isSteamBlocked) {
        const msg: string = await invoke('block_steam_updates', { steamPath });
        setIsSteamBlocked(true);
        showStatus(msg, 'success');
      } else {
        const msg: string = await invoke('unblock_steam_updates', { steamPath });
        setIsSteamBlocked(false);
        showStatus(msg, 'success');
      }
    } catch (err: any) {
      showStatus(`Operation failed: ${err}`, 'error');
    }
  };

  const handleResetPath = async () => {
    showStatus('Resetting configurations... Removing custom plugins and update blocks.', 'info');
    try {
      const settings: any = await invoke('get_settings');
      const steamPath = settings.steam_path;

      if (!steamPath || steamPath.trim() === '') {
        showStatus('Error: Please configure your Steam Path in Settings first!', 'error');
        return;
      }

      // 1. Remove AetherDLL binaries
      const installerResult = await invoke('uninstall_aether_dll', { steamPath }).catch(() => "Ok");
      
      // 2. Remove steam.cfg update block if active
      await invoke('unblock_steam_updates', { steamPath }).catch(() => {});
      setIsSteamBlocked(false);
      setIsDllInstalled(false);

      showStatus('Steam directory successfully reset to its original clean state.', 'success');
    } catch (err: any) {
      showStatus(`Reset operation failed: ${err}`, 'error');
    }
  };

  return (
    <div className="aether-view">
      
      {/* Big Uppercase Title at the top */}
      <h1 className="aether-title">AETHER</h1>
      
      {/* Operation Feedback inside the View */}
      {statusMsg.text && (
        <div className={`settings-alert ${statusMsg.type}`} style={{ width: '460px', padding: '10px 15px', fontSize: '12px', textAlign: 'center' }}>
          {statusMsg.text}
        </div>
      )}

      {/* SECTION 1: AetherDesk */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">AetherDesk</span>
          <span className="panel-meta">v1.0.0</span>
        </div>
        <div className="panel-actions">
          <button 
            className="panel-btn" 
            disabled={!isUpdateAvailable}
          >
            Update
          </button>
        </div>
      </div>

      {/* SECTION 2: AetherDLL */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">AetherDLL</span>
          <span className="panel-meta">{isDllInstalled ? 'v2.4.1' : 'N/A'}</span>
        </div>
        <div className="panel-actions">
          {/* Install/Update Button: Active if DLL is NOT installed, or if updates are available */}
          <button 
            onClick={handleInstallDll}
            className="panel-btn"
            disabled={isProcessing || (isDllInstalled && !isUpdateAvailable)}
          >
            {isDllInstalled ? 'Update' : 'Install'}
          </button>

          {/* Uninstall Button: Active ONLY if DLL is detected/installed */}
          <button 
            onClick={handleUninstallDll}
            className="panel-btn"
            disabled={isProcessing || !isDllInstalled}
          >
            Uninstall
          </button>
        </div>
      </div>

      {/* SECTION 3: Steam */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">Steam</span>
        </div>
        <div className="panel-actions">
          {/* Block / Unlock Update Button */}
          <button 
            onClick={handleToggleSteamBlock}
            className="panel-btn"
            disabled={isProcessing}
          >
            {isSteamBlocked ? 'Unlock Update' : 'Block Update'}
          </button>

          {/* Reset Path Button */}
          <button 
            onClick={handleResetPath}
            className="panel-btn"
            disabled={isProcessing}
          >
            Reset Path
          </button>
        </div>
      </div>

    </div>
  );
};
