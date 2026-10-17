import { useCallback, useEffect, useState, memo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ResetSteamConfirmModal, UninstallDeskConfirmModal } from '../modals/UninstallDeskModal';
import { getSettings } from '../hooks/useSettings';
import { DllStatusInfo } from '../types/ui';
import { toErrorMessage } from '../util/errors';

interface AetherViewProps {
  isUpdateAvailable: boolean;
  isDeskUpdateAvailable: boolean;
  /** Installed AetherDesk version, resolved at app startup (passed down from
   * App.tsx) so the panel shows the correct value from its first render. */
  deskVersion: string;
  isDllUpdateTest: boolean;
  isDeskUpdateTest: boolean;
  onUpdateComplete: () => void;
  dllStatus: DllStatusInfo;
  onDllStatusChange: () => Promise<void>;
}

type StatusTone = 'info' | 'success' | 'error';

type UninstallStep = { kind: 'confirm' } | { kind: 'resetSteam'; deleteUserData: boolean };

/**
 * AETHER panel: AetherDesk and AetherDLL lifecycle (install/update/
 * uninstall), Steam update blocking and Steam path reset.
 *
 * Steam-side operations no longer receive the path from the UI: the backend
 * commands resolve it from settings (`commands::command_steam_path`), so only
 * orchestration remains here — no `withSteamPath`, which existed solely to
 * pass that parameter (docs/shared_contracts.md §8).
 */
export const AetherView = memo(function AetherView({
  isUpdateAvailable,
  isDeskUpdateAvailable,
  deskVersion,
  isDllUpdateTest,
  isDeskUpdateTest,
  onUpdateComplete,
  dllStatus,
  onDllStatusChange,
}: AetherViewProps) {
  const [statusMsg, setStatusMsg] = useState({ text: '', type: 'info' as StatusTone });
  const [isProcessing, setIsProcessing] = useState(false);
  const [uninstallStep, setUninstallStep] = useState<UninstallStep | null>(null);
  const [testUpdatesEnabled, setTestUpdatesEnabled] = useState(false);
  /** Reset Path gate: the irreversible-confirm popup. Only mounted while
   *  open so its 10-second countdown restarts on every open. */
  const [showResetConfirm, setShowResetConfirm] = useState(false);

  const showStatus = useCallback((text: string, type: StatusTone) => {
    setStatusMsg({ text, type });
    setTimeout(() => setStatusMsg({ text: '', type: 'info' }), 6000);
  }, []);

  const formatVersion = (value: string) => {
    if (!value || value === 'N/A') return 'N/A';
    if (value === '…') return '…'; // still resolving
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

  useEffect(() => {
    // The desk version comes from App.tsx (resolved at startup via a local IPC,
    // no network), so the panel shows the correct value from its first render.
    // "Restore" replaces "Uninstall" while test updates are enabled.
    getSettings()
      .then((settings) => setTestUpdatesEnabled(Boolean(settings.enable_test_updates)))
      .catch(() => {});
  }, [isUpdateAvailable, isDeskUpdateAvailable]);

  const handleInstallDeskUpdate = async () => {
    setIsProcessing(true);
    showStatus('Preparing AetherDesk portable update...', 'info');

    try {
      const result: string = await invoke('install_aether_desk_update');
      showStatus(result, 'success');
      setIsProcessing(false);
    } catch (err: unknown) {
      showStatus(`AetherDesk update failed: ${toErrorMessage(err)}`, 'error');
      await onDllStatusChange();
      onUpdateComplete();
      setIsProcessing(false);
    }
  };

  /**
   * Schedules the external uninstaller helper and exits.
   * `deleteUserData` controls whether AetherData is deleted or kept in place.
   */
  const finishUninstall = async (deleteUserData: boolean) => {
    showStatus(
      deleteUserData
        ? 'Removing AetherDesk and user data...'
        : 'Removing verified AetherDesk files (keeping AetherData in place)...',
      'info'
    );
    await invoke('uninstall_aether_desk', { deleteUserData });
  };

  /** Optional Reset Path + unblock, then real folder uninstall. */
  const cleanSteamThenUninstall = async (deleteUserData: boolean) => {
    // The backend commands resolve the Steam path (backend-side settings):
    // only the two calls remain here, in the previous order.
    await invoke('reset_aether_steam_path');
    await invoke('unblock_steam_updates').catch(() => {});
    await refreshAfterDllChange().catch(() => {});
    await finishUninstall(deleteUserData);
  };

  // ----- Uninstall flow ----------------------------------------------------
  // 1. Uninstall click → confirm modal (checkbox for user data)
  // 2. YES → probe Steam residuals
  //    - residuals > 0 → Reset-Steam-path confirm (the same popup as the
  //      Reset Path button, countdown included)
  //    - else → finishUninstall immediately
  // 3. Reset YES → Reset Path then finishUninstall
  //    Reset NO  → finishUninstall only
  // Cancel / Esc / overlay at any step aborts without touching disk.

  const handleUninstallDesk = () => {
    if (isProcessing) return;
    setUninstallStep({ kind: 'confirm' });
  };

  // Test updates mode: the Uninstall button becomes Restore. Restore downloads
  // and reinstalls the latest stable AetherDesk build (the non-test version),
  // then restarts the app to apply it.
  const handleRestoreStable = async () => {
    if (isProcessing) return;
    setIsProcessing(true);
    showStatus('Restoring the stable AetherDesk build...', 'info');
    try {
      const result: string = await invoke('restore_stable_desk');
      showStatus(result, 'success');
    } catch (err: unknown) {
      showStatus(`Restore failed: ${toErrorMessage(err)}`, 'error');
      setIsProcessing(false);
    }
  };

  const handleUninstallConfirm = async (deleteUserData: boolean) => {
    setIsProcessing(true);
    try {
      // The probe is read-only and degrades to 0 when Steam is unconfigured
      // or unreachable: no need to read settings first to decide whether to
      // call it (one IPC + one DPAPI decryption saved).
      const residualCount = await invoke<number>('probe_aether_steam_residuals');

      if (residualCount > 0) {
        setUninstallStep({ kind: 'resetSteam', deleteUserData });
        setIsProcessing(false);
        return;
      }

      setUninstallStep(null);
      await finishUninstall(deleteUserData);
    } catch (err: unknown) {
      showStatus(`AetherDesk uninstall failed: ${toErrorMessage(err)}`, 'error');
      setIsProcessing(false);
      setUninstallStep(null);
    }
  };

  const handleUninstallResetYes = async () => {
    if (!uninstallStep || uninstallStep.kind !== 'resetSteam') return;
    const { deleteUserData } = uninstallStep;
    setIsProcessing(true);
    showStatus('Resetting the Steam path, then uninstalling AetherDesk...', 'info');
    try {
      setUninstallStep(null);
      await cleanSteamThenUninstall(deleteUserData);
    } catch (err: unknown) {
      showStatus(`Steam reset before uninstall failed: ${toErrorMessage(err)}`, 'error');
      setIsProcessing(false);
    }
  };

  const handleUninstallResetNo = async () => {
    if (!uninstallStep || uninstallStep.kind !== 'resetSteam') return;
    const { deleteUserData } = uninstallStep;
    setIsProcessing(true);
    try {
      setUninstallStep(null);
      await finishUninstall(deleteUserData);
    } catch (err: unknown) {
      showStatus(`AetherDesk uninstall failed: ${toErrorMessage(err)}`, 'error');
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
      const result = await invoke<string>('install_aether_dll');
      showStatus(result, 'success');
      await refreshAfterDllChange();
    } catch (err: unknown) {
      showStatus(`Installation failed: ${toErrorMessage(err)}`, 'error');
    } finally {
      setIsProcessing(false);
    }
  };

  const handleUninstallDll = async () => {
    setIsProcessing(true);
    showStatus('Removing AetherDLL binaries...', 'info');

    try {
      const result = await invoke<string>('uninstall_aether_dll');
      showStatus(result, 'success');
      await refreshAfterDllChange();
    } catch (err: unknown) {
      showStatus(`Uninstall failed: ${toErrorMessage(err)}`, 'error');
    } finally {
      setIsProcessing(false);
    }
  };

  const handleToggleSteamBlock = async () => {
    try {
      const msg = dllStatus.isSteamBlocked
        ? await invoke<string>('unblock_steam_updates')
        : await invoke<string>('block_steam_updates');
      await onDllStatusChange();
      showStatus(msg, 'success');
    } catch (err: unknown) {
      showStatus(`Operation failed: ${toErrorMessage(err)}`, 'error');
    }
  };

  const handleResetPath = async () => {
    setShowResetConfirm(false);
    showStatus('Resetting configurations... Removing custom plugins and update blocks.', 'info');
    try {
      const result = await (async () => {
        const msg = await invoke<string>('reset_aether_steam_path');
        await invoke('unblock_steam_updates').catch(() => {});
        return msg;
      })();
      await refreshAfterDllChange();
      showStatus(result, 'success');
    } catch (err: unknown) {
      showStatus(`Reset operation failed: ${toErrorMessage(err)}`, 'error');
    }
  };

  return (
    <div className="aether-view">
      <h1 className="aether-title">AETHER</h1>

      {statusMsg.text && (
        <div
          className={`settings-alert ${statusMsg.type} settings-alert--compact`}
          style={{ width: '460px', textAlign: 'center' }}
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
                title={isDeskUpdateTest ? 'AetherDesk TEST update is ready!' : 'AetherDesk update is ready!'}
              ></span>
            )}
          </button>

          <button
            onClick={testUpdatesEnabled ? handleRestoreStable : handleUninstallDesk}
            className="panel-btn"
            disabled={isProcessing}
            title={testUpdatesEnabled ? 'Restore the stable build (leave test channel)' : undefined}
          >
            {testUpdatesEnabled ? 'Restore' : 'Uninstall'}
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
            {dllStatus.isInstalled && isUpdateAvailable ? 'Update' : dllStatus.isInstalled ? 'Updated' : 'Install'}
            {dllStatus.isInstalled && isUpdateAvailable && (
              <span
                className={`btn-update-dot${isDllUpdateTest ? ' test' : ''}`}
                title={isDllUpdateTest ? 'AetherDLL TEST update is ready!' : 'AetherDLL update is ready!'}
              ></span>
            )}
          </button>

          <button onClick={handleUninstallDll} className="panel-btn" disabled={isProcessing || !dllStatus.isInstalled}>
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

          <button
            onClick={() => {
              if (isProcessing) return;
              setShowResetConfirm(true);
            }}
            className="panel-btn"
            disabled={isProcessing}
          >
            Reset Path
          </button>
        </div>
      </div>

      {showResetConfirm && (
        <ResetSteamConfirmModal
          isProcessing={isProcessing}
          onConfirm={() => void handleResetPath()}
          onCancel={() => {
            if (isProcessing) return;
            setShowResetConfirm(false);
          }}
        />
      )}

      {uninstallStep?.kind === 'confirm' && (
        <UninstallDeskConfirmModal
          isProcessing={isProcessing}
          onConfirm={handleUninstallConfirm}
          onCancel={cancelUninstall}
        />
      )}

      {uninstallStep?.kind === 'resetSteam' && (
        <ResetSteamConfirmModal
          isProcessing={isProcessing}
          onConfirm={handleUninstallResetYes}
          onReject={handleUninstallResetNo}
          onCancel={cancelUninstall}
        />
      )}
    </div>
  );
});
