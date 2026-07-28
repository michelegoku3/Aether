import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { requireSteamPath } from '../hooks/useSettings';

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

interface SpecificVersionModalProps {
  game: SpecificVersionGame;
  initialRows: LuaManifestRow[];
  onClose: () => void;
}

export const SpecificVersionModal = ({ game, initialRows, onClose }: SpecificVersionModalProps) => {
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

  const updateRow = (rowId: number, patch: Partial<LuaManifestRow>) => {
    setRows(prev => prev.map(row => row.rowId === rowId ? { ...row, ...patch } : row));
  };

  const handleOpenSteamDb = async () => {
    try {
      await invoke('open_steamdb_depots', { appId: Number(game.appId) });
    } catch (err: any) {
      setStatus({ text: `Failed to open SteamDB: ${err}`, type: 'error' });
    }
  };

  const handleApply = async () => {
    setIsApplying(true);
    setStatus({ text: 'Applying Lua manifest edits...', type: 'info' });

    try {
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

      // Successful apply is the user's confirmation. Close immediately to avoid
      // forcing a second click on X. If opened from Library, the parent Modify
      // popup remains mounted and becomes visible again.
      onClose();
    } catch (err: any) {
      setStatus({ text: `Failed to apply specific version edits: ${err}`, type: 'error' });
    } finally {
      setIsApplying(false);
    }
  };

  return (
    <div className="modal-overlay">
      <div className="modal-container version-modal-container">
        <div className="modal-header">
          <span className="modal-title">
            Specific Version: <strong style={{ color: '#ffffff' }}>{game.name}</strong> ({game.appId})
          </span>
          <button
            onClick={() => {
              if (!isApplying) {
                onClose();
              }
            }}
            className="modal-close-btn"
            disabled={isApplying}
            style={{ opacity: isApplying ? 0.3 : 1 }}
          >
            &times;
          </button>
        </div>

        <div className="modal-separator"></div>

        <div className="modal-body">
          {status.text && (
            <div className={`settings-alert ${status.type}`} style={{ padding: '10px 15px', fontSize: '12px' }}>
              {status.text}
            </div>
          )}

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
      </div>
    </div>
  );
};
