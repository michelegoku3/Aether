import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { LibraryActionGame } from './LibraryGameActionsModal';
import { openInFileManager } from '../util/paths';
import { useModalDismiss, useOverlayDismiss } from '../hooks/useModalDismiss';
import type { OnlineActionResult, OnlineEnableRequest, OnlinePlan, OnlineStatus } from '../types/online';
import { emptyRequest } from '../types/online';
import { OnlineDetectionSection } from './online/OnlineDetectionSection';
import { OnlineConfigSection } from './online/OnlineConfigSection';
import { Uco2CleanupModal } from './Uco2CleanupModal';

// Re-export all online domain types for backwards compatibility with any callers
export * from '../types/online';

interface OnlinePanelProps {
  game: LibraryActionGame;
  onClose: () => void;
}

export const OnlinePanel = ({ game, onClose }: OnlinePanelProps) => {
  const [plan, setPlan] = useState<OnlinePlan | null>(null);
  const [status, setStatus] = useState<OnlineStatus | null>(null);
  const [request, setRequest] = useState<OnlineEnableRequest>(() => emptyRequest(Number(game.appId) || 0));
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{
    text: string;
    kind: 'info' | 'success' | 'error';
  } | null>(null);
  // Journal-less Disable: confirmation text, then the verify-integrity warning.
  const [cleanupConfirm, setCleanupConfirm] = useState<string | null>(null);
  const [cleanupWarning, setCleanupWarning] = useState<string | null>(null);
  const [cleanupBusy, setCleanupBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const appId = Number(game.appId);
      const [planResult, statusResult, savedRequest] = await Promise.all([
        invoke<OnlinePlan>('plan_online', { appId }),
        invoke<OnlineStatus>('get_online_status', { appId }),
        invoke<OnlineEnableRequest | null>('get_online_preferences', { appId }),
      ]);
      setPlan(planResult);
      setStatus(statusResult);
      setRequest((previous) => {
        const next = savedRequest ?? {
          ...previous,
          ogAppId: previous.ogAppId || appId || 0,
        };
        if (!savedRequest && planResult.detection.steamstubDetected && !planResult.detection.steamlessApplied) {
          return { ...next, getStubbedLol: true };
        }
        return next;
      });
    } catch (err) {
      setMessage({ text: `Plan unavailable: ${err}`, kind: 'error' });
    } finally {
      setLoading(false);
    }
  }, [game.appId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useModalDismiss(onClose, busy || cleanupConfirm !== null || cleanupWarning !== null);
  const handleOverlayClick = useOverlayDismiss(onClose, busy || cleanupConfirm !== null || cleanupWarning !== null);

  const handleEnable = async () => {
    setBusy(true);
    setMessage({ text: 'Enabling...', kind: 'info' });
    try {
      const result = await invoke<OnlineActionResult>('enable_online', {
        appId: Number(game.appId),
        request,
      });
      setMessage({ text: result.message, kind: result.success ? 'success' : 'error' });
      await refresh();
    } catch (err) {
      setMessage({ text: `Enable failed: ${err}`, kind: 'error' });
    } finally {
      setBusy(false);
    }
  };

  const runDisable = async (confirmCleanup: boolean) => {
    setBusy(true);
    if (!confirmCleanup) setMessage({ text: 'Disabling...', kind: 'info' });
    try {
      const result = await invoke<OnlineActionResult>('disable_online', {
        appId: Number(game.appId),
        confirmCleanup,
      });
      if (result.confirmationRequired) {
        // Not an error: the journal is gone, so ask before deleting anything.
        setCleanupConfirm(result.message);
        setMessage(null);
        return;
      }
      if (result.warning) setCleanupWarning(result.warning);
      setMessage({ text: result.message, kind: result.success ? 'success' : 'error' });
      await refresh();
    } catch (err) {
      setMessage({ text: `Disable failed: ${err}`, kind: 'error' });
    } finally {
      setBusy(false);
    }
  };

  const confirmCleanup = async () => {
    setCleanupBusy(true);
    try {
      await runDisable(true);
      setCleanupConfirm(null);
    } finally {
      setCleanupBusy(false);
    }
  };

  const handleReset = async () => {
    const appId = Number(game.appId) || 0;
    try {
      await invoke('clear_online_preferences', { appId });
      setRequest(emptyRequest(appId));
      setMessage(null);
    } catch (err) {
      setMessage({ text: `Reset failed: ${err}`, kind: 'error' });
    }
  };

  const openFolder = async (folder: string) => {
    try {
      await openInFileManager(folder);
    } catch (err) {
      setMessage({ text: `Could not open the folder: ${err}`, kind: 'error' });
    }
  };

  const enabled = status?.state === 'enabled';
  const broken = status?.state === 'broken';
  const blocked = !plan?.prerequisites.bundleOk || (plan?.prerequisites.errors.length ?? 0) > 0;
  const bundleUpdate =
    enabled &&
    !!plan?.prerequisites.bundleVersion &&
    !!status?.record?.bundleVersion &&
    plan.prerequisites.bundleVersion !== status.record.bundleVersion;

  return (
    <>
      <div className="modal-overlay" onClick={handleOverlayClick}>
        <div className="modal-container op-container" onClick={(e) => e.stopPropagation()}>
          <div className="op-header">
            <h3 className="op-header-title">
              Enable Online: {game.name} ({game.appId})
            </h3>
            <button type="button" className="op-close" onClick={onClose} disabled={busy} aria-label="Close">
              ×
            </button>
          </div>

          <div className="op-body">
            {loading ? (
              <div className="op-muted">Analyzing the game...</div>
            ) : (
              <>
                {plan && <OnlineDetectionSection plan={plan} onOpenFolder={openFolder} />}

                {plan && (
                  <OnlineConfigSection plan={plan} request={request} enabled={enabled} setRequest={setRequest} />
                )}

                {/* Result message */}
                {message && <div className={`op-box op-message op-message--${message.kind}`}>{message.text}</div>}
              </>
            )}
          </div>

          <div className="op-footer">
            {bundleUpdate && (
              <button
                type="button"
                className={`modal-btn op-footer-btn op-btn${blocked || busy ? ' op-btn--disabled' : ''}`}
                onClick={handleEnable}
                disabled={blocked || busy}
                title={`Update deployed files to ${plan?.prerequisites.bundleVersion ?? 'the current bundle'}`}
              >
                {busy ? 'Updating...' : `Update ${plan?.prerequisites.bundleVersion ?? ''}`.trim()}
              </button>
            )}
            <button
              type="button"
              className={`modal-btn op-footer-btn ${enabled || broken ? 'op-btn--ghost' : 'op-btn'}${
                blocked || busy ? ' op-btn--disabled' : ''
              }`}
              onClick={enabled || broken ? () => runDisable(false) : handleEnable}
              disabled={enabled || broken ? busy : blocked || busy}
              title={!enabled && !broken && blocked ? 'Resolve the missing prerequisites first' : undefined}
            >
              {busy ? (enabled || broken ? 'Disabling...' : 'Enabling...') : enabled || broken ? 'Disable' : 'Enable'}
            </button>
            <button
              type="button"
              className={`modal-btn op-footer-btn op-btn--ghost${enabled || broken || busy ? ' op-btn--disabled' : ''}`}
              onClick={handleReset}
              disabled={enabled || broken || busy}
            >
              Reset
            </button>
          </div>
        </div>
      </div>

      {cleanupConfirm && (
        <Uco2CleanupModal
          mode="confirm"
          text={cleanupConfirm}
          busy={cleanupBusy}
          onConfirm={() => void confirmCleanup()}
          onClose={() => setCleanupConfirm(null)}
        />
      )}
      {cleanupWarning && (
        <Uco2CleanupModal mode="warning" text={cleanupWarning} onClose={() => setCleanupWarning(null)} />
      )}
    </>
  );
};
