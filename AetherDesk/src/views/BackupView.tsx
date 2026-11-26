import { useEffect, useMemo, useState, memo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useOptionalLibraryGames } from '../hooks/useLibraryGames';
import { toErrorMessage } from '../util/errors';

interface BackupGameSummary {
  app_id: number;
  game_name?: string | null;
  has_lua_original: boolean;
  history_count: number;
  manifest_count: number;
  has_crack: boolean;
  has_original_files: boolean;
  has_achievements: boolean;
  total_size_bytes: number;
  backup_path: string;
}

type BackupSortMode = 'name' | 'size';

const HISTORY_KEEP_LATEST = 3;

const formatBytes = (bytes: number): string => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
};

export const BackupView = memo(function BackupView() {
  const library = useOptionalLibraryGames();
  const [rows, setRows] = useState<BackupGameSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyAppId, setBusyAppId] = useState<number | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<number | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [sortMode, setSortMode] = useState<BackupSortMode>('name');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');

  const libraryNames = useMemo(() => {
    const map = new Map<number, string>();
    for (const game of library?.games ?? []) {
      const id = Number(game.appId);
      if (Number.isFinite(id) && game.name) {
        map.set(id, game.name);
      }
    }
    return map;
  }, [library?.games]);

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      const data = await invoke<BackupGameSummary[]>('list_backup_games');
      setRows(data || []);
    } catch (err) {
      setError(toErrorMessage(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const displayName = (row: BackupGameSummary): string =>
    row.game_name?.trim() || libraryNames.get(row.app_id) || `App ${row.app_id}`;

  const visibleRows = useMemo(() => {
    const needle = searchQuery.trim().toLowerCase();
    const filtered = rows.filter((row) => {
      if (!needle) return true;
      const name = (row.game_name?.trim() || libraryNames.get(row.app_id) || '').toLowerCase();
      return name.includes(needle) || String(row.app_id).includes(needle);
    });
    return [...filtered].sort((a, b) => {
      if (sortMode === 'size') {
        const diff = b.total_size_bytes - a.total_size_bytes;
        return diff !== 0 ? diff : a.app_id - b.app_id;
      }
      const nameA = a.game_name?.trim() || libraryNames.get(a.app_id) || `App ${a.app_id}`;
      const nameB = b.game_name?.trim() || libraryNames.get(b.app_id) || `App ${b.app_id}`;
      return nameA.localeCompare(nameB);
    });
  }, [rows, searchQuery, sortMode, libraryNames]);

  const totalBytes = useMemo(() => rows.reduce((sum, row) => sum + row.total_size_bytes, 0), [rows]);

  const openFolder = async (path: string) => {
    setError('');
    try {
      await invoke('reveal_in_file_manager', { path });
    } catch (err) {
      setError(toErrorMessage(err));
    }
  };

  const handlePruneHistory = async (appId: number) => {
    setBusyAppId(appId);
    setError('');
    setNotice('');
    try {
      const removed = await invoke<number>('prune_backup_lua_history', {
        appId,
        keepLatest: HISTORY_KEEP_LATEST,
      });
      setNotice(
        removed > 0
          ? `Pruned ${removed} old Lua history file(s) for App ID ${appId}.`
          : `No old Lua history files to prune for App ID ${appId}.`
      );
      await load();
    } catch (err) {
      setError(toErrorMessage(err));
    } finally {
      setBusyAppId(null);
    }
  };

  const handleDeleteBackup = async (appId: number) => {
    setBusyAppId(appId);
    setConfirmDeleteId(null);
    setError('');
    setNotice('');
    try {
      const msg = await invoke<string>('delete_game_backup', { appId });
      setNotice(msg);
      await load();
    } catch (err) {
      setError(toErrorMessage(err));
    } finally {
      setBusyAppId(null);
    }
  };

  return (
    <div className="backup-view-container" style={{ padding: '24px' }}>
      <div className="store-header">
        <h1 className="store-title">Backup</h1>
        <p className="store-subtitle">
          Per-game backups stored under AetherData/backup. Lua originals, version history, manifests, achievements,
          cracks and original files.
        </p>
      </div>
      <div className="store-separator" />
      <div className="backup-toolbar">
        <div className="home-search-wrapper" style={{ position: 'relative', width: '260px' }}>
          <input
            type="text"
            className="store-search-input"
            placeholder="Filter by game or App ID..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            style={{ width: '100%', paddingRight: '32px' }}
          />
          {searchQuery && (
            <button
              type="button"
              className="home-search-clear"
              aria-label="Clear backup filter"
              onClick={() => setSearchQuery('')}
            >
              &times;
            </button>
          )}
        </div>
        <select
          className="settings-select"
          aria-label="Sort backups"
          value={sortMode}
          onChange={(e) => setSortMode(e.target.value as BackupSortMode)}
          style={{ width: '130px' }}
        >
          <option value="name">Sort: Name</option>
          <option value="size">Sort: Size</option>
        </select>
        <button className="settings-small-btn" onClick={() => void load()} disabled={loading}>
          {loading ? 'Loading…' : 'Refresh'}
        </button>
        <span className="backup-summary-meta">
          {rows.length} {rows.length === 1 ? 'backup' : 'backups'} · {formatBytes(totalBytes)}
        </span>
      </div>
      {error && <div className="settings-alert error">{error}</div>}
      {notice && <div className="settings-alert success">{notice}</div>}
      {loading && rows.length === 0 ? (
        <div className="log-empty-state">Loading backups…</div>
      ) : rows.length === 0 ? (
        <div className="log-empty-state">No backups yet. Install or apply a crack to create one.</div>
      ) : visibleRows.length === 0 ? (
        <div className="log-empty-state">No backups match &ldquo;{searchQuery.trim()}&rdquo;.</div>
      ) : (
        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '13px' }}>
          <thead>
            <tr>
              <th style={{ textAlign: 'left', padding: '8px' }}>Game</th>
              <th style={{ textAlign: 'right', padding: '8px' }}>Size</th>
              <th style={{ textAlign: 'center', padding: '8px' }}>Lua</th>
              <th style={{ textAlign: 'center', padding: '8px' }}>History</th>
              <th style={{ textAlign: 'center', padding: '8px' }}>Manifests</th>
              <th style={{ textAlign: 'center', padding: '8px' }}>Achievements</th>
              <th style={{ textAlign: 'center', padding: '8px' }}>Crack</th>
              <th style={{ textAlign: 'center', padding: '8px' }}>Originals</th>
              <th style={{ textAlign: 'right', padding: '8px' }}>Actions</th>
            </tr>
          </thead>
          <tbody>
            {visibleRows.map((r) => {
              const rowBusy = busyAppId === r.app_id;
              const confirmingDelete = confirmDeleteId === r.app_id;
              return (
                <tr key={r.app_id} style={{ borderTop: '1px solid var(--border, #333)' }}>
                  <td style={{ padding: '8px' }}>
                    <div className="backup-game-name">{displayName(r)}</div>
                    <div className="backup-game-appid">App ID: {r.app_id}</div>
                  </td>
                  <td style={{ textAlign: 'right', padding: '8px', whiteSpace: 'nowrap' }}>
                    {formatBytes(r.total_size_bytes)}
                  </td>
                  <td style={{ textAlign: 'center', padding: '8px' }}>{r.has_lua_original ? '✓' : '—'}</td>
                  <td style={{ textAlign: 'center', padding: '8px' }}>{r.history_count}</td>
                  <td style={{ textAlign: 'center', padding: '8px' }}>{r.manifest_count}</td>
                  <td style={{ textAlign: 'center', padding: '8px' }}>{r.has_achievements ? '✓' : '—'}</td>
                  <td style={{ textAlign: 'center', padding: '8px' }}>{r.has_crack ? '✓' : '—'}</td>
                  <td style={{ textAlign: 'center', padding: '8px' }}>{r.has_original_files ? '✓' : '—'}</td>
                  <td style={{ textAlign: 'right', padding: '8px' }}>
                    <div className="backup-row-actions">
                      <button
                        className="settings-small-btn"
                        disabled={rowBusy}
                        onClick={() => void openFolder(r.backup_path)}
                      >
                        Open
                      </button>
                      {r.history_count > HISTORY_KEEP_LATEST && (
                        <button
                          className="settings-small-btn"
                          disabled={rowBusy}
                          title={`Keep only the ${HISTORY_KEEP_LATEST} newest Lua history entries`}
                          onClick={() => void handlePruneHistory(r.app_id)}
                        >
                          Prune
                        </button>
                      )}
                      {confirmingDelete ? (
                        <>
                          <button
                            className="settings-small-btn backup-btn-danger"
                            disabled={rowBusy}
                            onClick={() => void handleDeleteBackup(r.app_id)}
                          >
                            Confirm
                          </button>
                          <button
                            className="settings-small-btn"
                            disabled={rowBusy}
                            onClick={() => setConfirmDeleteId(null)}
                          >
                            Cancel
                          </button>
                        </>
                      ) : (
                        <button
                          className="settings-small-btn backup-btn-danger"
                          disabled={rowBusy}
                          onClick={() => setConfirmDeleteId(r.app_id)}
                        >
                          Delete
                        </button>
                      )}
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
});
