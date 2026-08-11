import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { DllStatusInfo } from '../types/ui';

interface AetherViewProps {
  isUpdateAvailable: boolean;
  isDeskUpdateAvailable: boolean;
  onUpdateComplete: () => void; // Refresh update check in the parent
  dllStatus: DllStatusInfo;
  onDllStatusChange: () => Promise<void>;
}

export const AetherView = ({ isUpdateAvailable, isDeskUpdateAvailable, onUpdateComplete, dllStatus, onDllStatusChange }: AetherViewProps) => {
  const [deskVersion, setDeskVersion] = useState('1.0.0');
  
  const [statusMsg, setStatusMsg] = useState({ text: '', type: 'info' });
  const [isProcessing, setIsProcessing] = useState(false);

  const showStatus = (text: string, type: 'info' | 'success' | 'error') => {
    setStatusMsg({ text, type });
    setTimeout(() => setStatusMsg({ text: '', type: 'info' }), 6000);
  };

  const formatVersion = (value: string) => {
    if (!value || value === 'N/A') return 'N/A';
    const normalized = value.toLowerCase().replace(/^desk-/, '').replace(/^dll-/, '').replace(/^v/, '');
    return `v${normalized}`;
  };

  // Check only AetherDesk version on component load
  const checkDeskVersion = async () => {
    try {
      const deskInfo: any = await invoke('check_aether_desk_update');
      setDeskVersion(deskInfo.installed_version || 'N/A');
    } catch (err: any) {
      console.error("Failed to query AetherDesk update state:", err);
    }
  };

  useEffect(() => {
    checkDeskVersion();
  }, [isUpdateAvailable, isDeskUpdateAvailable]); // re-run if update availability changes

  const handleInstallDeskUpdate = async () => {
    setIsProcessing(true);
    showStatus('Preparing AetherDesk portable update...', 'info');

    try {
      const result: string = await invoke('install_aether_desk_update');
      showStatus(result, 'success');
      // The portable updater swaps files and restarts the app automatically.
      setIsProcessing(false);
    } catch (err: any) {
      showStatus(`AetherDesk update failed: ${err}`, 'error');
      await onDllStatusChange();
      onUpdateComplete();
      setIsProcessing(false);
    }
  };

  const handleUninstallDesk = async () => {
    setIsProcessing(true);
    showStatus('AetherDesk is portable — closing. Delete the folder to uninstall.', 'info');

    try {
      await invoke('uninstall_aether_desk');
    } catch (err: any) {
      showStatus(`AetherDesk uninstall failed: ${err}`, 'error');
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
      
      // Refresh DLL status and update availability
      await onDllStatusChange();
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
      
      // Refresh DLL status and update availability
      await onDllStatusChange();
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

      if (!dllStatus.isSteamBlocked) {
        const msg: string = await invoke('block_steam_updates', { steamPath });
        await onDllStatusChange();
        showStatus(msg, 'success');
      } else {
        const msg: string = await invoke('unblock_steam_updates', { steamPath });
        await onDllStatusChange();
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

      const result: string = await invoke('reset_aether_steam_path', { steamPath });
      await invoke('unblock_steam_updates', { steamPath }).catch(() => {});

      // Refresh DLL status and update availability
      await onDllStatusChange();
      onUpdateComplete(); // notify parent to refresh update status

      showStatus(result, 'success');
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
            {formatVersion(deskVersion)}
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

          <button
            onClick={handleUninstallDesk}
            className="panel-btn"
            disabled={isProcessing}
          >
            Uninstall
          </button>
        </div>
      </div>

      {/* SECTION 2: AetherDLL */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">AetherDLL</span>
          {/* Dynamically displays the actual local installed version read from
              the PE version resource inside the .dll files themselves */}
          <span className="panel-meta">{dllStatus.isInstalled ? formatVersion(dllStatus.installedVersion) : 'N/A'}</span>
        </div>
        <div className="panel-actions">
          {/* Install/Update Button: Active if DLL is NOT installed, or if updates are available */}
          <button 
            onClick={handleInstallDll}
            className="panel-btn"
            disabled={isProcessing || (dllStatus.isInstalled && !isUpdateAvailable)}
          >
            {dllStatus.isInstalled && isUpdateAvailable ? 'Update' : dllStatus.isInstalled ? 'Updated' : 'Install'}
            {/* Superimposed glowing update dot overlay directly inside the relative button container! */}
            {dllStatus.isInstalled && isUpdateAvailable && (
              <span className="btn-update-dot" title="AetherDLL update is ready!"></span>
            )}
          </button>

          {/* Uninstall Button: Active ONLY if DLL is detected/installed */}
          <button 
            onClick={handleUninstallDll}
            className="panel-btn"
            disabled={isProcessing || !dllStatus.isInstalled}
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
            {dllStatus.isSteamBlocked ? 'Unlock Update' : 'Block Update'}
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
