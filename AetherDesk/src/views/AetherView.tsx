import { useCallback, useEffect, useRef, useState, memo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ResetSteamConfirmModal, UninstallDeskConfirmModal } from '../modals/UninstallDeskModal';
import { SteamCloseConfirmModal } from '../modals/SteamCloseConfirmModal';
import { getSettings } from '../hooks/useSettings';
import { useDeskUpdateProgress } from '../hooks/useDeskUpdateProgress';
import { DllStatusInfo } from '../types/ui';
import type { SteamKillConsentArgs } from '../types/steam';
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
 * A Steam-side operation waiting for the "close Steam?" decision. The same
 * operation is re-run with `allowSteamKill` once the user accepts, so the kill
 * and the write happen inside one backend command.
 */
type SteamGatedAction =
  | { kind: 'installDll' }
  | { kind: 'uninstallDll' }
  | { kind: 'resetPath' }
  | { kind: 'uninstallDesk'; deleteUserData: boolean };

/**
 * AETHER panel: AetherDesk and AetherDLL lifecycle (install/update/
 * uninstall), Steam update blocking and Steam path reset.
 *
 * Steam-side operations take no path from the UI: the backend commands
 * resolve it from settings (`commands::command_steam_path`), so only
 * orchestration remains here (docs/shared_contracts.md §8).
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
  /** True from the UPDATE click until the command returns: the progress
   *  listener only lives for the duration of one update. */
  const [isDeskUpdating, setIsDeskUpdating] = useState(false);
  const deskUpdateProgress = useDeskUpdateProgress(isDeskUpdating);
  /** Reset Path gate: the irreversible-confirm popup. Only mounted while
   *  open so its 10-second countdown restarts on every open. */
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  /** Set when Steam is open and the pending operation needs a decision. */
  const [steamGate, setSteamGate] = useState<SteamGatedAction | null>(null);
  /** Synchronous guard, read before the probe resolves: React state alone
   *  cannot stop a second click from starting a second destructive run. */
  const busyRef = useRef(false);

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
    // "Restore" replaces "Uninstall" while test updates are enabled.
    getSettings()
      .then((settings) => setTestUpdatesEnabled(Boolean(settings.enable_test_updates)))
      .catch(() => {});
  }, [isUpdateAvailable, isDeskUpdateAvailable, isDeskUpdateTest]);

  const handleInstallDeskUpdate = async () => {
    setIsProcessing(true);
    setIsDeskUpdating(true);
    showStatus('Preparing AetherDesk portable update...', 'info');

    try {
      const result: string = await invoke('install_aether_desk_update');
      showStatus(result, 'success');
      setIsProcessing(false);
    } catch (err: unknown) {
      // Drop the live line before the status refresh: a stale "Downloading..."
      // would otherwise sit over the failure alert while the DLL check runs.
      setIsDeskUpdating(false);
      showStatus(`AetherDesk update failed: ${toErrorMessage(err)}`, 'error');
      await onDllStatusChange();
      onUpdateComplete();
      setIsProcessing(false);
    } finally {
      setIsDeskUpdating(false);
    }
  };

  /**
   * Live line for the portable update: phase and elapsed seconds. The bar above
   * carries the progress. `null` until the first event, so the static
   * "Preparing..." text still covers the gap before the backend answers.
   */
  const deskUpdateLine = (() => {
    if (!isDeskUpdating) return null;
    // The first backend event can be a moment away, and the 6 s status alert
    // above expires: without this fallback the panel goes blank mid-update.
    if (!deskUpdateProgress) return 'Preparing AetherDesk portable update...';
    const { phase, elapsedSeconds } = deskUpdateProgress;
    const seconds = `${elapsedSeconds}s`;

    switch (phase) {
      case 'looking-up':
        return `Looking for a newer AetherDesk build (${seconds})`;
      case 'downloading':
        return `Downloading AetherDesk (${seconds})`;
      case 'extracting':
        return `Unpacking the update (${seconds})`;
      case 'staging':
        return `Preparing the restart (${seconds})`;
      case 'restarting':
        return 'Restarting AetherDesk to apply the update...';
    }
  })();

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
  const cleanSteamThenUninstall = async (deleteUserData: boolean, allowSteamKill: boolean) => {
    showStatus('Resetting the Steam path, then uninstalling AetherDesk...', 'info');
    try {
      // The backend commands resolve the Steam path from settings: only the two
      // Steam calls remain here, in this order.
      const args: SteamKillConsentArgs = { allowSteamKill };
      await invoke('reset_aether_steam_path', args);
      await invoke('unblock_steam_updates').catch(() => {});
      await refreshAfterDllChange().catch(() => {});
      await finishUninstall(deleteUserData);
    } catch (err: unknown) {
      showStatus(`Could not reset Steam and uninstall AetherDesk: ${toErrorMessage(err)}`, 'error');
    }
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
    setIsDeskUpdating(true);
    showStatus('Restoring the stable AetherDesk build...', 'info');
    try {
      const result: string = await invoke('restore_stable_desk');
      showStatus(result, 'success');
    } catch (err: unknown) {
      showStatus(`Restore failed: ${toErrorMessage(err)}`, 'error');
      setIsProcessing(false);
    }
    setIsDeskUpdating(false);
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
    setUninstallStep(null);
    await requestSteamGatedAction({ kind: 'uninstallDesk', deleteUserData });
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
  //
  // Every operation that writes into the Steam folder fails while Steam holds
  // its files. When Steam is open we ask first (SteamCloseConfirmModal); YES
  // re-runs the same action with `allowSteamKill`, so the backend terminates
  // Steam and performs the write in one command. NO normally only closes the
  // popup; on the AetherDesk uninstall path it skips the optional Steam clean
  // and keeps uninstalling, mirroring "No, just uninstall".

  const installDll = async (allowSteamKill: boolean) => {
    showStatus('Fetching latest release from GitHub...', 'info');
    const args: SteamKillConsentArgs = { allowSteamKill };

    try {
      const result = await invoke<string>('install_aether_dll', args);
      showStatus(result, 'success');
      await refreshAfterDllChange();
    } catch (err: unknown) {
      showStatus(`Installation failed: ${toErrorMessage(err)}`, 'error');
    }
  };

  const uninstallDll = async (allowSteamKill: boolean) => {
    showStatus('Removing AetherDLL binaries...', 'info');
    const args: SteamKillConsentArgs = { allowSteamKill };

    try {
      const result = await invoke<string>('uninstall_aether_dll', args);
      showStatus(result, 'success');
      await refreshAfterDllChange();
    } catch (err: unknown) {
      showStatus(`Uninstall failed: ${toErrorMessage(err)}`, 'error');
    }
  };

  const resetSteamPath = async (allowSteamKill: boolean) => {
    showStatus('Resetting configurations... Removing custom plugins and update blocks.', 'info');
    const args: SteamKillConsentArgs = { allowSteamKill };
    try {
      const result = await (async () => {
        const msg = await invoke<string>('reset_aether_steam_path', args);
        await invoke('unblock_steam_updates').catch(() => {});
        return msg;
      })();
      await refreshAfterDllChange();
      showStatus(result, 'success');
    } catch (err: unknown) {
      showStatus(`Reset operation failed: ${toErrorMessage(err)}`, 'error');
    }
  };

  const runSteamGatedAction = async (action: SteamGatedAction, allowSteamKill: boolean) => {
    switch (action.kind) {
      case 'installDll':
        return installDll(allowSteamKill);
      case 'uninstallDll':
        return uninstallDll(allowSteamKill);
      case 'resetPath':
        return resetSteamPath(allowSteamKill);
      case 'uninstallDesk':
        return cleanSteamThenUninstall(action.deleteUserData, allowSteamKill);
    }
  };

  /**
   * Locks the panel for the whole decision and run. The ref is the real guard
   * (it is read before the probe answers, when state has not rendered yet);
   * `isProcessing` only drives the disabled buttons.
   */
  const holdBusy = () => {
    busyRef.current = true;
    setIsProcessing(true);
  };

  const releaseBusy = () => {
    busyRef.current = false;
    setIsProcessing(false);
  };

  const requestSteamGatedAction = async (action: SteamGatedAction) => {
    if (busyRef.current) return;
    holdBusy();

    let steamRunning = false;
    try {
      // Fresh scan: the cached monitor can be a second stale, and the popup has
      // to appear exactly when the command would otherwise refuse.
      steamRunning = await invoke<boolean>('probe_steam_running');
    } catch {
      // Advisory probe: without an answer the command's own guard still decides.
    }
    if (steamRunning) {
      // Stay locked while the user decides; YES/NO/X release the guard.
      setSteamGate(action);
      return;
    }
    try {
      await runSteamGatedAction(action, false);
    } finally {
      releaseBusy();
    }
  };

  const handleSteamGateConfirm = async () => {
    if (!steamGate) return;
    const action = steamGate;
    setSteamGate(null);
    try {
      await runSteamGatedAction(action, true);
    } finally {
      releaseBusy();
    }
  };

  /** X / Escape / overlay: dismiss and abort, nothing runs. */
  const handleSteamGateCancel = () => {
    setSteamGate(null);
    releaseBusy();
  };

  /** NO on the uninstall path: skip the Steam clean, still remove AetherDesk. */
  const handleSteamGateSkipClean = async () => {
    if (!steamGate || steamGate.kind !== 'uninstallDesk') return;
    const { deleteUserData } = steamGate;
    setSteamGate(null);
    try {
      await finishUninstall(deleteUserData);
    } catch (err: unknown) {
      showStatus(`AetherDesk uninstall failed: ${toErrorMessage(err)}`, 'error');
    } finally {
      releaseBusy();
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
    await requestSteamGatedAction({ kind: 'resetPath' });
  };

  return (
    <div className="aether-view">
      <h1 className="aether-title">AETHER</h1>

      {deskUpdateLine ? (
        <div className="version-progress aether-update-progress">
          <div className="version-progress-label" id="aether-desk-update-label">
            {deskUpdateLine}
          </div>
          <div
            className="version-progress-bar"
            role="progressbar"
            aria-label="AetherDesk update"
            aria-describedby="aether-desk-update-label"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={deskUpdateProgress ? deskUpdateProgress.progress : undefined}
          >
            <div className="version-progress-fill" style={{ width: `${deskUpdateProgress?.progress ?? 0}%` }} />
          </div>
        </div>
      ) : (
        statusMsg.text && (
          <div
            className={`settings-alert ${statusMsg.type} settings-alert--compact`}
            style={{ width: '460px', textAlign: 'center' }}
          >
            {statusMsg.text}
          </div>
        )
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
            onClick={() => void requestSteamGatedAction({ kind: 'installDll' })}
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

          <button
            onClick={() => void requestSteamGatedAction({ kind: 'uninstallDll' })}
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

      {steamGate && (
        <SteamCloseConfirmModal
          onConfirm={() => void handleSteamGateConfirm()}
          onCancel={handleSteamGateCancel}
          onReject={steamGate.kind === 'uninstallDesk' ? () => void handleSteamGateSkipClean() : undefined}
        />
      )}
    </div>
  );
});
