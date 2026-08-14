import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  UninstallDeskConfirmModal,
  UninstallSteamCleanModal,
} from '../modals/UninstallDeskModal';
import { getSettings, requireSteamPath } from '../hooks/useSettings';
import { DllStatusInfo } from '../types/ui';

interface AetherViewProps {
  isUpdateAvailable: boolean;
  isDeskUpdateAvailable: boolean;
  isDllUpdateTest: boolean;
  isDeskUpdateTest: boolean;
  onUpdateComplete: () => void;
  dllStatus: DllStatusInfo;
  onDllStatusChange: () => Promise<void>;
}

type StatusTone = 'info' | 'success' | 'error';

type UninstallStep =
  | { kind: 'confirm' }
  | { kind: 'steamClean'; residualCount: number; deleteUserData: boolean };

/**
 * Steam-side operations that share the same steam_path prerequisite.
 * Keeps AetherView focused on orchestration, not path plumbing.
 */
const withSteamPath = async <T,>(
  run: (steamPath: string) => Promise<T>,
): Promise<T> => {
  const steamPath = await requireSteamPath();
  return run(steamPath);
};

export const AetherView = ({
  isUpdateAvailable,
  isDeskUpdateAvailable,
  isDllUpdateTest,
  isDeskUpdateTest,
  onUpdateComplete,
  dllStatus,
  onDllStatusChange,
}: AetherViewProps) => {
  const [deskVersion, setDeskVersion] = useState('1.0.0');
  const [statusMsg, setStatusMsg] = useState({ text: '', type: 'info' as StatusTone });
  const [isProcessing, setIsProcessing] = useState(false);
  const [uninstallStep, setUninstallStep] = useState<UninstallStep | null>(null);

  const showStatus = useCallback((text: string, type: StatusTone) => {
    setStatusMsg({ text, type });
    setTimeout(() => setStatusMsg({ text: '', type: 'info' }), 6000);
  }, []);

  const formatVersion = (value: string) => {
    if (!value || value === 'N/A') return 'N/A';
    const normalized = value
      .toLowerCase()
      .replace(/^desk-/, '')
      .replace(/^dll-/, '')
      .replace(/^v/, '');
    return `v${normalized}`;
  };

  const refreshAfterDllChange = async () => {
    await onDllStatusChange();
    onUpdateComplete();
  };

  const checkDeskVersion = async () => {
    try {
      const deskInfo: any = await invoke('check_aether_desk_update');
      setDeskVersion(deskInfo.installed_version || 'N/A');
    } catch (err: any) {
      console.error('Failed to query AetherDesk update state:', err);
    }
  };

  useEffect(() => {
    void checkDeskVersion();
  }, [isUpdateAvailable, isDeskUpdateAvailable]);

  const handleInstallDeskUpdate = async () => {
    setIsProcessing(true);
    showStatus('Preparing AetherDesk portable update...', 'info');

    try {
      const result: string = await invoke('install_aether_desk_update');
      showStatus(result, 'success');
      setIsProcessing(false);
    } catch (err: any) {
      showStatus(`AetherDesk update failed: ${err}`, 'error');
      await onDllStatusChange();
      onUpdateComplete();
      setIsProcessing(false);
    }
  };

  /**
   * Schedules the external uninstaller helper and exits.
   * `deleteUserData` controls whether AetherData is wiped or relocated.
   */
  const finishUninstall = async (deleteUserData: boolean) => {
    showStatus(
      deleteUserData
        ? 'Removing AetherDesk and user data...'
        : 'Removing AetherDesk (keeping AetherData next to the folder)...',
      'info',
    );
    await invoke('uninstall_aether_desk', { deleteUserData });
  };

  /** Optional Reset Path + unblock, then real folder uninstall. */
  const cleanSteamThenUninstall = async (deleteUserData: boolean) => {
    await withSteamPath(async (steamPath) => {
      await invoke('reset_aether_steam_path', { steamPath });
      await invoke('unblock_steam_updates', { steamPath }).catch(() => {});
    });
    await refreshAfterDllChange().catch(() => {});
    await finishUninstall(deleteUserData);
  };

  // ----- Uninstall flow ----------------------------------------------------
  // 1. Uninstall click → confirm modal (checkbox for user data)
  // 2. YES → probe Steam residuals
  //    - residuals > 0 → steam-clean modal
  //    - else → finishUninstall immediately
  // 3. Steam YES → Reset Path then finishUninstall
  //    Steam NO  → finishUninstall only
  // Cancel / Esc / overlay at any step aborts without touching disk.

  const handleUninstallDesk = () => {
    if (isProcessing) return;
    setUninstallStep({ kind: 'confirm' });
  };

  const handleUninstallConfirm = async (deleteUserData: boolean) => {
    setIsProcessing(true);
    try {
      const settings = await getSettings();
      const steamPath = (settings.steam_path || '').trim();
      let residualCount = 0;

      if (steamPath) {
        residualCount = await invoke<number>('probe_aether_steam_residuals', { steamPath });
      }

      if (residualCount > 0) {
        setUninstallStep({ kind: 'steamClean', residualCount, deleteUserData });
        setIsProcessing(false);
        return;
      }

      setUninstallStep(null);
      await finishUninstall(deleteUserData);
    } catch (err: any) {
      showStatus(`AetherDesk uninstall failed: ${err}`, 'error');
      setIsProcessing(false);
      setUninstallStep(null);
    }
  };

  const handleSteamCleanYes = async () => {
    if (!uninstallStep || uninstallStep.kind !== 'steamClean') return;
    const { deleteUserData } = uninstallStep;
    setIsProcessing(true);
    showStatus('Cleaning Steam (Reset Path), then uninstalling AetherDesk...', 'info');
    try {
      setUninstallStep(null);
      await cleanSteamThenUninstall(deleteUserData);
    } catch (err: any) {
      showStatus(`Steam clean before uninstall failed: ${err}`, 'error');
      setIsProcessing(false);
    }
  };

  const handleSteamCleanNo = async () => {
    if (!uninstallStep || uninstallStep.kind !== 'steamClean') return;
    const { deleteUserData } = uninstallStep;
    setIsProcessing(true);
    try {
      setUninstallStep(null);
      await finishUninstall(deleteUserData);
    } catch (err: any) {
      showStatus(`AetherDesk uninstall failed: ${err}`, 'error');
      setIsProcessing(false);
    }
  };

  const cancelUninstall = () => {
    if (isProcessing) return;
    setUninstallStep(null);
  };

  // ----- DLL / Steam actions -----------------------------------------------

  const handleInstallDll = async () => {
    setIsProcessing(true);
    showStatus('Fetching latest release from GitHub...', 'info');

    try {
      const result: string = await withSteamPath((steamPath) =>
        invoke('install_aether_dll', { steamPath }),
      );
      showStatus(result, 'success');
      await refreshAfterDllChange();
    } catch (err: any) {
      showStatus(`Installation failed: ${err}`, 'error');
    } finally {
      setIsProcessing(false);
    }
  };

  const handleUninstallDll = async () => {
    setIsProcessing(true);
    showStatus('Removing AetherDLL binaries...', 'info');

    try {
      const result: string = await withSteamPath((steamPath) =>
        invoke('uninstall_aether_dll', { steamPath }),
      );
      showStatus(result, 'success');
      await refreshAfterDllChange();
    } catch (err: any) {
      showStatus(`Uninstall failed: ${err}`, 'error');
    } finally {
      setIsProcessing(false);
    }
  };

  const handleToggleSteamBlock = async () => {
    try {
      const msg: string = await withSteamPath(async (steamPath) => {
        if (!dllStatus.isSteamBlocked) {
          return invoke('block_steam_updates', { steamPath });
        }
        return invoke('unblock_steam_updates', { steamPath });
      });
      await onDllStatusChange();
      showStatus(msg, 'success');
    } catch (err: any) {
      showStatus(`Operation failed: ${err}`, 'error');
    }
  };

  const handleResetPath = async () => {
    showStatus('Resetting configurations... Removing custom plugins and update blocks.', 'info');
    try {
      const result: string = await withSteamPath(async (steamPath) => {
        const msg: string = await invoke('reset_aether_steam_path', { steamPath });
        await invoke('unblock_steam_updates', { steamPath }).catch(() => {});
        return msg;
      });
      await refreshAfterDllChange();
      showStatus(result, 'success');
    } catch (err: any) {
      showStatus(`Reset operation failed: ${err}`, 'error');
    }
  };

  return (
    <div className="aether-view">
      <h1 className="aether-title">AETHER</h1>

      {statusMsg.text && (
        <div
          className={`settings-alert ${statusMsg.type}`}
          style={{ width: '460px', padding: '10px 15px', fontSize: '12px', textAlign: 'center' }}
        >
          {statusMsg.text}
        </div>
      )}

      {/* SECTION 1: AetherDesk */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">AetherDesk</span>
          <span className="panel-meta">{formatVersion(deskVersion)}</span>
        </div>
        <div className="panel-actions">
          <button
            onClick={handleInstallDeskUpdate}
            className="panel-btn"
            disabled={isProcessing || !isDeskUpdateAvailable}
          >
            {isDeskUpdateAvailable ? 'Update' : 'Updated'}
            {isDeskUpdateAvailable && (
              <span
                className={`btn-update-dot${isDeskUpdateTest ? ' test' : ''}`}
                title={
                  isDeskUpdateTest
                    ? 'AetherDesk TEST update is ready!'
                    : 'AetherDesk update is ready!'
                }
              ></span>
            )}
          </button>

          <button onClick={handleUninstallDesk} className="panel-btn" disabled={isProcessing}>
            Uninstall
          </button>
        </div>
      </div>

      {/* SECTION 2: AetherDLL */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">AetherDLL</span>
          <span className="panel-meta">
            {dllStatus.isInstalled ? formatVersion(dllStatus.installedVersion) : 'N/A'}
          </span>
        </div>
        <div className="panel-actions">
          <button
            onClick={handleInstallDll}
            className="panel-btn"
            disabled={isProcessing || (dllStatus.isInstalled && !isUpdateAvailable)}
          >
            {dllStatus.isInstalled && isUpdateAvailable
              ? 'Update'
              : dllStatus.isInstalled
                ? 'Updated'
                : 'Install'}
            {dllStatus.isInstalled && isUpdateAvailable && (
              <span
                className={`btn-update-dot${isDllUpdateTest ? ' test' : ''}`}
                title={
                  isDllUpdateTest
                    ? 'AetherDLL TEST update is ready!'
                    : 'AetherDLL update is ready!'
                }
              ></span>
            )}
          </button>

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
          <button onClick={handleToggleSteamBlock} className="panel-btn" disabled={isProcessing}>
            {dllStatus.isSteamBlocked ? 'Unlock Update' : 'Block Update'}
          </button>

          <button onClick={handleResetPath} className="panel-btn" disabled={isProcessing}>
            Reset Path
          </button>
        </div>
      </div>

      {uninstallStep?.kind === 'confirm' && (
        <UninstallDeskConfirmModal
          isProcessing={isProcessing}
          onConfirm={handleUninstallConfirm}
          onCancel={cancelUninstall}
        />
      )}

      {uninstallStep?.kind === 'steamClean' && (
        <UninstallSteamCleanModal
          residualCount={uninstallStep.residualCount}
          isProcessing={isProcessing}
          onConfirmClean={handleSteamCleanYes}
          onSkipClean={handleSteamCleanNo}
          onCancel={cancelUninstall}
        />
      )}
    </div>
  );
};
