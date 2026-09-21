import { useCallback, useEffect, useState, memo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  UninstallDeskConfirmModal,
  UninstallSteamCleanModal,
} from '../modals/UninstallDeskModal';
import { SyncStatusModal } from '../modals/SyncStatusModal';
import { ActivityIcon } from '../ui/icons';
import { getSettings } from '../hooks/useSettings';
import { DllStatusInfo } from '../types/ui';

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

type UninstallStep =
  | { kind: 'confirm' }
  | { kind: 'steamClean'; residualCount: number; deleteUserData: boolean };

/**
 * Pannello AETHER: ciclo di vita di AetherDesk e AetherDLL (install/update/
 * uninstall), blocco degli aggiornamenti di Steam, reset del percorso Steam e
 * diagnostica del sincronizzatore in background.
 *
 * Le operazioni lato Steam non ricevono più il percorso dalla UI: lo risolvono
 * i comandi backend dalle impostazioni (`commands::command_steam_path`), quindi
 * qui resta solo l'orchestrazione — niente `withSteamPath`, che esisteva unicamente
 * per passare quel parametro (docs/shared_contracts.md §8).
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
  /** Background-sync popup (pin sync / pin refresh / repair / workshop lanes).
   *  Lives here, not in Library: it is plant diagnostics about the
   *  synchronizer, not an action on a single game. */
  const [showSyncStatus, setShowSyncStatus] = useState(false);

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
    // Il percorso Steam lo risolvono i comandi (settings lato backend): qui
    // restano solo le due chiamate, nell'ordine di prima.
    await invoke('reset_aether_steam_path');
    await invoke('unblock_steam_updates').catch(() => {});
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
    } catch (err: any) {
      showStatus(`Restore failed: ${err}`, 'error');
      setIsProcessing(false);
    }
  };

  const handleUninstallConfirm = async (deleteUserData: boolean) => {
    setIsProcessing(true);
    try {
      // Il probe è read-only e degrada a 0 quando Steam non è configurato o
      // non è raggiungibile: non serve più leggere le impostazioni per
      // decidere se chiamarlo (una IPC + una decifratura DPAPI in meno).
      const residualCount = await invoke<number>('probe_aether_steam_residuals');

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
      const result = await invoke<string>('install_aether_dll');
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
      const result = await invoke<string>('uninstall_aether_dll');
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
      const msg = dllStatus.isSteamBlocked
        ? await invoke<string>('unblock_steam_updates')
        : await invoke<string>('block_steam_updates');
      await onDllStatusChange();
      showStatus(msg, 'success');
    } catch (err: any) {
      showStatus(`Operation failed: ${err}`, 'error');
    }
  };

  const handleResetPath = async () => {
    showStatus('Resetting configurations... Removing custom plugins and update blocks.', 'info');
    try {
      const result = await (async () => {
        const msg = await invoke<string>('reset_aether_steam_path');
        await invoke('unblock_steam_updates').catch(() => {});
        return msg;
      })();
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
                title={
                  isDeskUpdateTest
                    ? 'AetherDesk TEST update is ready!'
                    : 'AetherDesk update is ready!'
                }
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

      {/* SECTION 4: Background synchronizer */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">Synchronizer</span>
        </div>
        <div className="panel-actions">
          <button
            disabled
            className="panel-btn panel-btn--icon"
            title="Not available yet"
          >
            <ActivityIcon />
            Background Sync
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

      {showSyncStatus && <SyncStatusModal onClose={() => setShowSyncStatus(false)} />}
    </div>
  );
});
