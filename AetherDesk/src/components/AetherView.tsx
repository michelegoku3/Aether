import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface AetherViewProps {
  isUpdateAvailable: boolean;
  isDeskUpdateAvailable: boolean;
  onUpdateComplete: () => void; // Refresh update check in the parent
}

export const AetherView = ({ isUpdateAvailable, isDeskUpdateAvailable, onUpdateComplete }: AetherViewProps) => {
  // Real active states bound to local filesystem and config status
  const [isDllInstalled, setIsDllInstalled] = useState(false);
  const [installedVersion, setInstalledVersion] = useState('N/A');
  const [deskVersion, setDeskVersion] = useState('1.0.0');
  const [latestDeskVersion, setLatestDeskVersion] = useState('N/A');
  const [isSteamBlocked, setIsSteamBlocked] = useState(false);
  
  const [statusMsg, setStatusMsg] = useState({ text: '', type: 'info' });
  const [isProcessing, setIsProcessing] = useState(false);

  const showStatus = (text: string, type: 'info' | 'success' | 'error') => {
    setStatusMsg({ text, type });
    setTimeout(() => setStatusMsg({ text: '', type: 'info' }), 6000);
  };

  // Perform active check on local system state on component load
  const checkLocalSystemState = async () => {
    try {
      const deskInfo: any = await invoke('check_aether_desk_update');
      setDeskVersion(deskInfo.installed_version || 'N/A');
      setLatestDeskVersion(deskInfo.latest_version || 'N/A');
    } catch (err: any) {
      console.error("Failed to query AetherDesk update state:", err);
    }

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

        // 4. Query backend to verify local installed version dynamically
        const updateInfo: any = await invoke('check_aether_dll_update', { steamPath });
        setInstalledVersion(updateInfo.installed_version);
      }
    } catch (err: any) {
      console.error("Failed to query local system state:", err);
    }
  };

  useEffect(() => {
    checkLocalSystemState();
  }, [isUpdateAvailable, isDeskUpdateAvailable]); // re-run if update availability changes

  const handleInstallDeskUpdate = async () => {
    setIsProcessing(true);
    showStatus('Preparing native AetherDesk update...', 'info');

    try {
      const result: string = await invoke('install_aether_desk_update');
      showStatus(result, 'success');
      // The Rust updater restarts the app after a successful native install.
      setIsProcessing(false);
    } catch (err: any) {
      showStatus(`AetherDesk update failed: ${err}`, 'error');
      await checkLocalSystemState();
      onUpdateComplete();
      setIsProcessing(false);
    }
  };

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
      
      showStatus(result, 'success');
      
      // Refresh local files and version states on completion
      await checkLocalSystemState();
      onUpdateComplete(); // notify parent to refresh update status
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
      
      showStatus(result, 'success');
      
      // Refresh local states
      await checkLocalSystemState();
      onUpdateComplete(); // notify parent to refresh update status
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

      // 1. Remove AetherDLL binaries and version files
      await invoke('uninstall_aether_dll', { steamPath }).catch(() => "Ok");
      
      // 2. Remove steam.cfg update block if active
      await invoke('unblock_steam_updates', { steamPath }).catch(() => {});
      
      // Refresh local state representation
      await checkLocalSystemState();
      onUpdateComplete(); // notify parent to refresh update status

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
          <span className="panel-meta">
            v{deskVersion}{isDeskUpdateAvailable && latestDeskVersion !== 'N/A' ? ` → v${latestDeskVersion}` : ''}
          </span>
        </div>
        <div className="panel-actions">
          <button 
            onClick={handleInstallDeskUpdate}
            className="panel-btn" 
            disabled={isProcessing || !isDeskUpdateAvailable}
          >
            {isDeskUpdateAvailable ? 'Update' : 'Updated'}
            {isDeskUpdateAvailable && (
              <span className="btn-update-dot" title="AetherDesk update is ready!"></span>
            )}
          </button>
        </div>
      </div>

      {/* SECTION 2: AetherDLL */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">AetherDLL</span>
          {/* Dynamically displays the actual local installed version from steam directory */}
          <span className="panel-meta">{isDllInstalled ? installedVersion : 'N/A'}</span>
        </div>
        <div className="panel-actions">
          {/* Install/Update Button: Active if DLL is NOT installed, or if updates are available */}
          <button 
            onClick={handleInstallDll}
            className="panel-btn"
            disabled={isProcessing || (isDllInstalled && !isUpdateAvailable)}
          >
            {isDllInstalled && isUpdateAvailable ? 'Update' : isDllInstalled ? 'Installed' : 'Install'}
            {/* Superimposed glowing update dot overlay directly inside the relative button container! */}
            {isDllInstalled && isUpdateAvailable && (
              <span className="btn-update-dot" title="AetherDLL update is ready!"></span>
            )}
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
