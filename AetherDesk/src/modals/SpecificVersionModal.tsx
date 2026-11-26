import React, { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { LuaManifestIssue } from '../hooks/useLibraryGames';
import { useModalDismiss } from '../hooks/useModalDismiss';
import { useWatchdog } from '../hooks/useWatchdog';
import { useLibraryGames } from '../hooks/useLibraryGames';
import { ModalShell } from '../ui/ModalShell';
import { StatusAlert } from '../ui/StatusAlert';
import { StatusMessage } from '../types/ui';
import { toErrorMessage } from '../util/errors';

export type DepotHostSkipReason = 'wrongOs' | 'wrongArch' | 'steamDeckOnly';

export interface DepotHostSkip {
  reason: DepotHostSkipReason;
  label: string;
  detail: string;
}

export interface LuaManifestRow {
  rowId: number;
  appId: number;
  manifestId: string;
  enabled: boolean;
  manifestInput?: string;
  /**
   * Set when this row is a malformed line rather than an editable pin: the
   * file cannot load in AetherDLL until it is repaired, and the row is here so
   * the editor can offer the repair instead of refusing to open.
   */
  issue?: LuaManifestIssue;
  /**
   * Set when Steam's cached AppInfo indicates this depot does not target this
   * host (different OS, 32-bit sibling alongside 64-bit, or Steam Deck only).
   */
  hostSkip?: DepotHostSkip;
}

/**
 * Prepares backend rows for the editor.
 *
 * A malformed row gets the value that is actually in the file (`manifestId`
 * carries the raw text, e.g. a hex GID) inside its input field: the field that
 * holds the wrong value is the one that must be seen and corrected, and leaving
 * it as a grey placeholder would hide exactly what has to change.
 */
export const prepareManifestRows = (rows: LuaManifestRow[] | null | undefined): LuaManifestRow[] =>
  (rows || []).map((row) => ({
    ...row,
    manifestInput: row.issue ? row.manifestId : row.manifestInput || '',
  }));

/** Same rule as the backend (`ensure_decimal_uint64`): digits only, u64 range. */
const isValidManifestGid = (value: string): boolean => {
  if (!/^[0-9]{1,20}$/.test(value)) return false;
  return BigInt(value) <= 18446744073709551615n;
};

export interface SpecificVersionGame {
  name: string;
  appId: string;
}

interface ManualVersionEditorProps {
  game: SpecificVersionGame;
  initialRows: LuaManifestRow[];
  onClose: () => void;
}

/** Uniform confirmation dialog before unlocking a non-host depot row for editing. */
const NonHostDepotConfirmModal = ({
  row,
  onConfirm,
  onCancel,
}: {
  row: LuaManifestRow;
  onConfirm: () => void;
  onCancel: () => void;
}) => (
  <ModalShell
    title={`Non-applicable depot (${row.hostSkip?.label ?? 'Other host'})`}
    onClose={onCancel}
    containerClassName="uninstall-modal"
    bodyClassName="uninstall-modal-body"
  >
    <p className="uninstall-modal-lead">Depot {row.appId} is not used on this PC.</p>
    <p className="uninstall-modal-copy">
      {row.hostSkip?.detail ?? 'Steam and AetherDLL skip this depot on this system.'} Editing its manifest ID or toggle
      is normally unnecessary.
    </p>
    <div className="uninstall-modal-actions">
      <button type="button" className="uninstall-btn uninstall-btn-primary" onClick={onConfirm}>
        Edit anyway
      </button>
      <button type="button" className="uninstall-btn uninstall-btn-secondary" onClick={onCancel}>
        Cancel
      </button>
    </div>
  </ModalShell>
);

/** The per-depot manifest editor, hosted by the Change Version modal as its "Manual" tab. */
export const ManualVersionEditor = ({ game, initialRows, onClose }: ManualVersionEditorProps) => {
  const [rows, setRows] = useState<LuaManifestRow[]>(prepareManifestRows(initialRows));
  // Baseline = the state we consider "unchanged". Starts as the rows passed by
  // the caller, then is replaced by what is actually on disk once loaded, so a
  // build applied in the Auto tab (or any external edit) becomes the new
  // baseline instead of being flagged as a manual change.
  const [baseline, setBaseline] = useState<LuaManifestRow[]>(initialRows);
  const [unlockedRowIds, setUnlockedRowIds] = useState<Set<number>>(() => new Set());
  const [pendingUnlockRow, setPendingUnlockRow] = useState<LuaManifestRow | null>(null);
  const [status, setStatus] = useState<StatusMessage>(
    initialRows.length > 0
      ? { text: 'Lua ready. Edit manifest IDs or disable depots, then apply.', type: 'info' }
      : { text: 'Lua ready, but no editable setManifestid entries were found.', type: 'error' }
  );
  const [isApplying, setIsApplying] = useState(false);
  const { arm: armWatchdog, clear: clearWatchdog } = useWatchdog();
  const { loadInstalledGames, queryGameState } = useLibraryGames();

  // Refresh the rows from disk on mount: the Manual tab must show the live Lua
  // state (e.g. the manifests just written by an apply in the Auto tab), not a
  // snapshot captured when the popup opened. Served through the shared provider
  // cache, so the StrictMode double mount does not read the Lua twice.
  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const fresh = await queryGameState<LuaManifestRow[]>(Number(game.appId), 'get_installed_lua_manifest_rows');
        if (cancelled) return;
        const normalized = prepareManifestRows(fresh);
        setRows(normalized);
        setBaseline(normalized);
        if (normalized.length === 0) {
          setStatus({
            text: 'Lua ready, but no editable setManifestid entries were found.',
            type: 'error',
          });
        } else if (normalized.some((row) => row.issue && row.issue.active)) {
          setStatus({
            text: 'This Lua contains a malformed pin (marked in red): AetherDLL ignores the whole file until it is repaired. Type a valid decimal manifest GID and press Apply Edits, or turn the row off to comment it out.',
            type: 'error',
          });
        } else if (normalized.some((row) => row.issue)) {
          setStatus({
            text: 'This Lua contains a commented-out malformed pin (marked in amber). The file loads now, but that call would break it as soon as updates are enabled for this game.',
            type: 'info',
          });
        }
      } catch {
        // Keep the rows passed by the caller if the refresh fails.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [game.appId]);

  const updateRow = (rowId: number, patch: Partial<LuaManifestRow>) => {
    setRows((prev) => prev.map((row) => (row.rowId === rowId ? { ...row, ...patch } : row)));
  };

  // Apply Edits must stay disabled while nothing differs from the Lua file:
  // the backend treats an empty input as "keep the original manifest ID", so
  // only a typed (non-empty, different) manifest ID or a disabled depot is a
  // real edit worth writing.
  const hasChanges = useMemo(() => {
    return rows.some((row) => {
      const original = baseline.find((r) => r.rowId === row.rowId);
      const enabledChanged = row.enabled !== (original ? original.enabled : row.enabled);
      const typed = row.manifestInput?.trim() ?? '';
      const manifestChanged = typed.length > 0 && typed !== (original?.manifestId ?? row.manifestId);
      return enabledChanged || manifestChanged;
    });
  }, [rows, baseline]);

  // Typing is validated here as well as in the backend: the same "digits only,
  // u64" rule means Apply can stay disabled on an obviously wrong value
  // instead of failing after the filesystem round-trip.
  const invalidInputRows = useMemo(
    () =>
      rows.filter((row) => {
        const typed = row.manifestInput?.trim() ?? '';
        return typed.length > 0 && !isValidManifestGid(typed);
      }),
    [rows]
  );

  // Escape closes only when nothing would be lost and no stacked confirm popup
  // is open: while NonHostDepotConfirmModal is open, Escape must close only it.
  useModalDismiss(onClose, isApplying || hasChanges || pendingUnlockRow !== null);

  const handleOpenSteamDb = async () => {
    try {
      await invoke('open_steamdb_depots', { appId: Number(game.appId) });
    } catch (err: unknown) {
      setStatus({ text: `Failed to open SteamDB: ${toErrorMessage(err)}`, type: 'error' });
    }
  };

  const handleApply = async () => {
    if (isApplying) {
      return;
    }
    setIsApplying(true);
    setStatus({ text: 'Applying Lua manifest edits...', type: 'info' });

    try {
      // Watchdog: the edit command is synchronous and fast on the backend.
      // If it does not settle, stop the spinner and show a real message
      // instead of loading forever.
      armWatchdog(() => {
        setStatus({
          text: 'The edit is taking longer than expected. Check the Logs view for details.',
          type: 'error',
        });
        setIsApplying(false);
      }, 30_000);

      const edits = rows.map((row) => ({
        rowId: row.rowId,
        manifestId: row.manifestInput?.trim() ? row.manifestInput.trim() : null,
        enabled: row.enabled,
      }));

      await invoke('apply_specific_version_edits', {
        appId: Number(game.appId),
        edits,
      });

      // Keep this direct user flow immediately consistent even if the pushed
      // invalidation is delayed; the provider coalesces duplicate requests.
      loadInstalledGames();
      clearWatchdog();

      // Successful apply is the user's confirmation. Close immediately to avoid
      // forcing a second click on X. If opened from Library, the parent Modify
      // popup remains mounted and becomes visible again.
      onClose();
    } catch (err: unknown) {
      clearWatchdog();
      setStatus({ text: `Failed to apply specific version edits: ${toErrorMessage(err)}`, type: 'error' });
    } finally {
      clearWatchdog();
      setIsApplying(false);
    }
  };

  return (
    <>
      <StatusAlert status={status} className="settings-alert--compact" />

      <div className="version-table-wrapper">
        <table className="version-table">
          <thead>
            <tr>
              <th>App ID</th>
              <th>setManifest ID</th>
              <th>Enabled</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => {
              const typed = (row.manifestInput ?? '').trim();
              const typedInvalid = typed.length > 0 && !isValidManifestGid(typed);
              // Red = the file does not load (active malformed call, or a value
              // the user just typed that the runtime would reject).
              // Amber = a commented malformed call: a trap for "enable updates",
              // not a broken load today.
              const severity: 'ok' | 'warn' | 'invalid' =
                typedInvalid || row.issue?.active ? 'invalid' : row.issue ? 'warn' : 'ok';
              const isNonHost = Boolean(row.hostSkip) && !row.issue;
              const isHostLocked = isNonHost && !unlockedRowIds.has(row.rowId);
              return (
                <tr
                  key={row.rowId}
                  className={[
                    !row.enabled ? 'disabled' : '',
                    severity === 'invalid' ? 'version-row-invalid' : '',
                    severity === 'warn' ? 'version-row-warn' : '',
                    isNonHost ? 'version-row-nonhost' : '',
                  ]
                    .filter(Boolean)
                    .join(' ')}
                >
                  <td className="version-appid" title={isNonHost ? row.hostSkip?.detail : undefined}>
                    <span className="version-appid-value">{row.appId}</span>
                    {isNonHost && row.hostSkip && <span className="version-host-badge">{row.hostSkip.label}</span>}
                  </td>
                  <td>
                    <input
                      className={[
                        'version-manifest-input',
                        severity === 'invalid' ? 'version-manifest-input--invalid' : '',
                        severity === 'warn' ? 'version-manifest-input--warn' : '',
                        isHostLocked ? 'version-manifest-input--nonhost-locked' : '',
                      ]
                        .filter(Boolean)
                        .join(' ')}
                      value={row.manifestInput || ''}
                      placeholder={row.manifestId}
                      readOnly={isHostLocked}
                      disabled={isApplying || (!row.enabled && !row.issue && !isHostLocked)}
                      aria-invalid={severity === 'invalid'}
                      title={isHostLocked ? row.hostSkip?.detail : undefined}
                      onClick={() => {
                        if (isHostLocked) setPendingUnlockRow(row);
                      }}
                      onKeyDown={(e) => {
                        if (!isHostLocked) return;
                        if (e.key === 'Tab' || e.key === 'Escape' || e.key === 'Shift') return;
                        e.preventDefault();
                        setPendingUnlockRow(row);
                      }}
                      onPaste={(e) => {
                        if (isHostLocked) {
                          e.preventDefault();
                          setPendingUnlockRow(row);
                        }
                      }}
                      onChange={(e) => {
                        if (isHostLocked) return;
                        updateRow(row.rowId, { manifestInput: e.target.value });
                      }}
                    />
                  </td>
                  <td className="version-switch-cell">
                    <label
                      className={['version-switch', isHostLocked ? 'version-switch--nonhost-locked' : '']
                        .filter(Boolean)
                        .join(' ')}
                      title={isHostLocked ? row.hostSkip?.detail : undefined}
                      tabIndex={!isApplying && isHostLocked ? 0 : undefined}
                      onClick={() => {
                        if (!isApplying && isHostLocked) setPendingUnlockRow(row);
                      }}
                      onKeyDown={(e) => {
                        if (isApplying || !isHostLocked) return;
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault();
                          setPendingUnlockRow(row);
                        }
                      }}
                    >
                      <input
                        type="checkbox"
                        checked={row.enabled}
                        disabled={isApplying || isHostLocked}
                        onChange={(e) => {
                          if (isHostLocked) return;
                          updateRow(row.rowId, { enabled: e.target.checked });
                        }}
                      />
                      <span></span>
                    </label>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <div className="version-actions">
        <button className="panel-btn" onClick={handleOpenSteamDb} disabled={isApplying}>
          Open SteamDB
        </button>
        <button
          className="panel-btn"
          onClick={handleApply}
          disabled={isApplying || rows.length === 0 || !hasChanges || invalidInputRows.length > 0}
        >
          {isApplying ? 'Applying...' : 'Apply Edits'}
        </button>
      </div>

      {pendingUnlockRow && (
        <NonHostDepotConfirmModal
          row={pendingUnlockRow}
          onConfirm={() => {
            setUnlockedRowIds((prev) => new Set(prev).add(pendingUnlockRow.rowId));
            setPendingUnlockRow(null);
          }}
          onCancel={() => setPendingUnlockRow(null)}
        />
      )}
    </>
  );
};
