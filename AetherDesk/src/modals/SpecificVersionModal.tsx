import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { requireSteamPath } from '../hooks/useSettings';
import { useModalDismiss } from '../hooks/useModalDismiss';

// Geometry used to size the depot table to its content: each row is ~41px
// (10px padding ×2 + 17px text + 1px border) and the sticky header ~37px.
const VERSION_ROW_HEIGHT_PX = 41;
const VERSION_HEADER_HEIGHT_PX = 37;
// Vertical space consumed by the modal chrome around the table: header,
// separator, status alert, actions row and paddings.
const VERSION_MODAL_CHROME_PX = 300;
const VERSION_TABLE_MIN_PX = 140;

export interface LuaManifestRow {
  rowId: number;
  appId: number;
  manifestId: string;
  enabled: boolean;
  manifestInput?: string;
}

export interface SpecificVersionGame {
  name: string;
  appId: string;
}

interface ManualVersionEditorProps {
  game: SpecificVersionGame;
  initialRows: LuaManifestRow[];
  onClose: () => void;
}

/**
 * The classic per-depot manifest editor (the former "Specific Version" UI).
 * Extracted so the Change Version modal can host it as its "Manual" tab.
 */
export const ManualVersionEditor = ({ game, initialRows, onClose }: ManualVersionEditorProps) => {
  const [rows, setRows] = useState<LuaManifestRow[]>(
    initialRows.map(row => ({ ...row, manifestInput: row.manifestInput || '' }))
  );
  const [status, setStatus] = useState({
    text: initialRows.length > 0
      ? 'Lua ready. Edit manifest IDs or disable depots, then apply.'
      : 'Lua ready, but no editable setManifestid entries were found.',
    type: initialRows.length > 0 ? 'info' : 'error'
  });
  const [isApplying, setIsApplying] = useState(false);
  const watchdogRef = React.useRef<number | null>(null);

  const clearWatchdog = () => {
    if (watchdogRef.current !== null) {
      window.clearTimeout(watchdogRef.current);
      watchdogRef.current = null;
    }
  };

  // Clear any pending watchdog when the editor unmounts.
  React.useEffect(() => clearWatchdog, []);

  // Dynamic modal height: grows with the number of editable depots, capped at
  // what fits the current window (the table scrolls when many depots exist).
  const tableMaxHeight = Math.max(
    VERSION_TABLE_MIN_PX,
    Math.min(
      rows.length * VERSION_ROW_HEIGHT_PX + VERSION_HEADER_HEIGHT_PX,
      Math.max(VERSION_TABLE_MIN_PX, window.innerHeight - VERSION_MODAL_CHROME_PX)
    )
  );

  const updateRow = (rowId: number, patch: Partial<LuaManifestRow>) => {
    setRows(prev => prev.map(row => row.rowId === rowId ? { ...row, ...patch } : row));
  };

  // ESC + click fuori chiudono il popup (rispettando un'operazione in corso).
  useModalDismiss(onClose, isApplying);

  const handleOpenSteamDb = async () => {
    try {
      await invoke('open_steamdb_depots', { appId: Number(game.appId) });
    } catch (err: any) {
      setStatus({ text: `Failed to open SteamDB: ${err}`, type: 'error' });
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
      watchdogRef.current = window.setTimeout(() => {
        setStatus({
          text: 'The edit is taking longer than expected. Check the Logs view for details.',
          type: 'error',
        });
        setIsApplying(false);
      }, 30_000);

      const steamPath = await requireSteamPath();

      const edits = rows.map(row => ({ 
        rowId: row.rowId,
        manifestId: row.manifestInput?.trim() ? row.manifestInput.trim() : null,
        enabled: row.enabled,
      }));

      await invoke('apply_specific_version_edits', {
        appId: Number(game.appId),
        steamPath,
        edits,
      });

      clearWatchdog();

      // Successful apply is the user's confirmation. Close immediately to avoid
      // forcing a second click on X. If opened from Library, the parent Modify
      // popup remains mounted and becomes visible again.
      onClose();
    } catch (err: any) {
      clearWatchdog();
      setStatus({ text: `Failed to apply specific version edits: ${err}`, type: 'error' });
    } finally {
      clearWatchdog();
      setIsApplying(false);
    }
  };

  return (
    <div className="modal-body">
      {status.text && (
        <div className={`settings-alert ${status.type}`} style={{ padding: '10px 15px', fontSize: '12px' }}>
          {status.text}
        </div>
      )}

      <div
        className="version-table-wrapper"
        style={{ '--version-table-max-height': `${tableMaxHeight}px` } as React.CSSProperties}
      >
        <table className="version-table">
          <thead>
            <tr>
              <th>App ID</th>
              <th>setManifest ID</th>
              <th>Enabled</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(row => (
              <tr key={row.rowId} className={!row.enabled ? 'disabled' : ''}>
                <td className="version-appid">{row.appId}</td>
                <td>
                  <input
                    className="version-manifest-input"
                    value={row.manifestInput || ''}
                    placeholder={row.manifestId}
                    disabled={isApplying || !row.enabled}
                    onChange={(e) => updateRow(row.rowId, { manifestInput: e.target.value })}
                  />
                </td>
                <td className="version-switch-cell">
                  <label className="version-switch">
                    <input
                      type="checkbox"
                      checked={row.enabled}
                      disabled={isApplying}
                      onChange={(e) => updateRow(row.rowId, { enabled: e.target.checked })}
                    />
                    <span></span>
                  </label>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="version-actions">
        <button
          className="panel-btn"
          onClick={handleOpenSteamDb}
          disabled={isApplying}
        >
          Open SteamDB
        </button>
        <button
          className="panel-btn"
          onClick={handleApply}
          disabled={isApplying || rows.length === 0}
        >
          {isApplying ? 'Applying...' : 'Apply Edits'}
        </button>
      </div>
    </div>
  );
};

interface SpecificVersionModalProps {
  game: SpecificVersionGame;
  initialRows: LuaManifestRow[];
  onClose: () => void;
}

/**
 * Backwards-compatible standalone modal wrapping the manual editor. New
 * callers should use `ChangeVersionModal` instead, which hosts this editor
 * as one of its tabs.
 */
const SpecificVersionModal = ({ game, initialRows, onClose }: SpecificVersionModalProps) => (
  <div className="modal-overlay" onClick={onClose}>
    <div
      className="modal-container version-modal-container"
      onClick={(e) => e.stopPropagation()}
    >
      <div className="modal-header">
        <span className="modal-title">
          Specific Version: <strong style={{ color: '#ffffff' }}>{game.name}</strong> ({game.appId})
        </span>
        <button onClick={onClose} className="modal-close-btn">&times;</button>
      </div>

      <div className="modal-separator"></div>

      <ManualVersionEditor game={game} initialRows={initialRows} onClose={onClose} />
    </div>
  </div>
);

export default SpecificVersionModal;
