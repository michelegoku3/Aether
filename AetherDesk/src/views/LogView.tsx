import { useEffect, useState, useRef, useMemo, memo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useVisiblePolling } from '../hooks/useVisiblePolling';

const LEVEL_RANK: Record<string, number> = {
  trace: 0,
  debug: 1,
  info: 2,
  warn: 3,
  error: 4,
  off: 5,
};

/** How often to poll when the Logs tab is visible and focused. */
const LOG_POLL_INTERVAL_MS = 2500;
/** Back-off interval when the tab is hidden (document.hidden) or the view
 *  is not the active tab — we still refresh occasionally so coming back to
 *  logs after a while doesn't show a stale dump, but we don't hammer IPC. */
const LOG_POLL_BACKOFF_MS = 10_000;
/** Debounce for the free-text filter so typing doesn't re-filter the whole
 *  window on every keystroke. */
const FILTER_DEBOUNCE_MS = 150;
/** Window of log lines requested from the backend and the cap on how many of
 *  the filtered ones are rendered: the newest 1000. Large enough to cover a
 *  whole troubleshooting session without exporting the file, and still cheap
 *  to render (the filter walks the array once, newest first). */
const TAIL_LINES = 1_000;
const MAX_RENDER_LINES = TAIL_LINES;

const lineRank = (line: string) => {
  if (line.includes('[ERROR]')) return 4;
  if (line.includes('[WARN ]') || line.includes('[WARN]')) return 3;
  if (line.includes('[INFO ]') || line.includes('[INFO]')) return 2;
  if (line.includes('[DEBUG]')) return 1;
  if (line.includes('[TRACE]')) return 0;
  return 2;
};

/** Cheap deterministic key for a line that stays stable across re-renders.
 *  Lines don't carry a server-assigned id, so we use the line content plus
 *  its index within the current snapshot. When a new line is appended the
 *  existing rendered nodes keep their keys and React doesn't remount them. */
const lineKey = (line: string, idx: number, snapshot: number) =>
  `${snapshot}:${idx}:${line.slice(0, 40)}:${line.length}`;

export const LogView = memo(function LogView() {
  const [lines, setLines] = useState<string[]>([]);
  const [filterQuery, setFilterQuery] = useState('');
  const [debouncedFilter, setDebouncedFilter] = useState('');
  const [logLevel, setLogLevel] = useState('trace');
  const [logSource, setLogSource] = useState<'desk' | 'dll' | 'uco2' | 'both'>('desk');
  const [exportStatus, setExportStatus] = useState('');
  const containerRef = useRef<HTMLDivElement | null>(null);
  const isAtBottomRef = useRef(true);
  /** Bumped every time we replace the `lines` array so the React keys are
   *  re-namespaced to the snapshot and a line appended at the tail doesn't
   *  collide with a previous index. */
  const snapshotRef = useRef(0);

  // Debounce the text filter.
  useEffect(() => {
    const id = window.setTimeout(() => setDebouncedFilter(filterQuery), FILTER_DEBOUNCE_MS);
    return () => window.clearTimeout(id);
  }, [filterQuery]);

  const fetchLogs = async () => {
    try {
      const recent: string[] = await invoke('get_recent_log_lines', {
        tailLines: TAIL_LINES,
        source: logSource,
      });
      setLines(recent || []);
      snapshotRef.current += 1;
    } catch (err) {
      console.warn('Failed to fetch logs:', err);
    }
  };

  /** Visibility-aware polling: fast cadence while the Logs tab is visible,
   *  slow cadence when the document is hidden (window minimised / another
   *  tab focused in the OS / another Aether view active). Shared hook: the
   *  same policy now drives the background-sync popup too, so there is one
   *  place where "how often do we poll" is decided. `resetKey: logSource`
   *  reloads immediately when the user switches Desk/DLL/UCO2/All. */
  useVisiblePolling(fetchLogs, {
    intervalMs: LOG_POLL_INTERVAL_MS,
    backoffMs: LOG_POLL_BACKOFF_MS,
    resetKey: logSource,
  });

  const handleScroll = () => {
    if (!containerRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = containerRef.current;
    isAtBottomRef.current = scrollHeight - (scrollTop + clientHeight) < 20;
  };

  useEffect(() => {
    if (isAtBottomRef.current && containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [lines]);

  const handleClearLogs = async () => {
    try {
      await invoke('clear_session_log', { source: logSource });
      await fetchLogs();
    } catch (err) {
      console.warn('Failed to clear logs:', err);
    }
  };

  const handleSaveBundle = async () => {
    try {
      const msg: string = await invoke('export_logs_bundle');
      setExportStatus(msg);
      setTimeout(() => setExportStatus(''), 5000);
    } catch (err: any) {
      setExportStatus(`Export failed: ${err}`);
      setTimeout(() => setExportStatus(''), 6000);
    }
  };

  // Memoised filter: recompute only when the source lines, the level or the
  // debounced query change. Lower-casing the needle once (not per-line) and
  // slicing the result to MAX_RENDER_LINES (most recent) caps the render work
  // during spammy builds.
  const filteredLines = useMemo(() => {
    if (logLevel === 'off') return [];
    const minRank = LEVEL_RANK[logLevel] ?? 0;
    const needle = debouncedFilter.trim().toLowerCase();
    const out: string[] = [];
    // Lines arrive newest-last from the backend; iterate in reverse to keep
    // the most recent matches when capping, then reverse back for display.
    for (let i = lines.length - 1; i >= 0 && out.length < MAX_RENDER_LINES; i--) {
      const line = lines[i];
      if (lineRank(line) < minRank) continue;
      if (needle && !line.toLowerCase().includes(needle)) continue;
      out.push(line);
    }
    return out.reverse();
  }, [lines, logLevel, debouncedFilter]);

  // Downloads the single log document for the selected source (desk/dll/uco2),
  // or the merged "all" document, with the same time-stamped naming as the .zip.
  const handleDownloadSource = async () => {
    try {
      const msg: string = await invoke('export_log_source', { source: logSource });
      setExportStatus(msg);
      setTimeout(() => setExportStatus(''), 5000);
    } catch (err: any) {
      setExportStatus(`Export failed: ${err}`);
      setTimeout(() => setExportStatus(''), 6000);
    }
  };

  const getLineClass = (line: string) => {
    if (line.includes('[ERROR]')) return 'log-line error';
    if (line.includes('[WARN ]') || line.includes('[WARN]')) return 'log-line warn';
    if (line.includes('[DEBUG]')) return 'log-line debug';
    if (line.includes('[TRACE]')) return 'log-line trace';
    return 'log-line info';
  };

  return (
    <div className="log-view-container">
      {/* Upper header section */}
      <div className="store-header">
        <h1 className="store-title">Logs</h1>
        <p className="store-subtitle">
          Real-time console monitoring for Desk, DLL and UCO2 logs.
        </p>
      </div>

      {/* Separator line */}
      <div className="store-separator"></div>

      {exportStatus && (
        <div
          className="settings-alert info"
          style={{ padding: '8px 14px', fontSize: '12px' }}
        >
          {exportStatus}
        </div>
      )}

      {/* Control bar */}
      <div className="log-header-controls" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexWrap: 'wrap', gap: '16px', width: '100%' }}>
        <div className="home-search-wrapper" style={{ position: 'relative', width: '260px' }}>
          <input
            type="text"
            placeholder="Filter logs by keyword..."
            value={filterQuery}
            onChange={(e) => setFilterQuery(e.target.value)}
            className="store-search-input"
            style={{ width: '100%', paddingRight: '36px' }}
          />
          {filterQuery && (
            <button
              type="button"
              className="home-search-clear"
              onClick={() => setFilterQuery('')}
            >
              &times;
            </button>
          )}
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: '12px', flexWrap: 'wrap' }}>
          <select
            className="settings-select"
            value={logSource}
            style={{ width: '105px', minWidth: '105px', height: '33px', padding: '0 8px', boxSizing: 'border-box' }}
            onChange={(e) => setLogSource(e.target.value as 'desk' | 'dll' | 'uco2' | 'both')}
            title="Select log source to view"
          >
            <option value="desk">Desk</option>
            <option value="dll">DLL</option>
            <option value="uco2">UCO2</option>
            <option value="both">All</option>
          </select>

          <select
            className="settings-select"
            value={logLevel}
            style={{ width: '110px', minWidth: '110px', height: '33px', padding: '0 8px', boxSizing: 'border-box' }}
            onChange={(e) => setLogLevel(e.target.value)}
            title="Filter visible lines by minimum level. TRACE is the default view; the live log level itself is not changed."
          >
            <option value="trace">TRACE</option>
            <option value="debug">DEBUG</option>
            <option value="info">INFO</option>
            <option value="warn">WARN</option>
            <option value="error">ERROR</option>
            <option value="off">OFF</option>
          </select>

          <button
            type="button"
            className="settings-small-btn"
            style={{ width: '80px', height: '33px', padding: '0', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box' }}
            onClick={handleSaveBundle}
            title="Export AetherDesk, AetherDLL and UCO2 logs as .zip in Downloads folder"
          >
            Save
          </button>

          <button
            type="button"
            className="settings-small-btn"
            style={{ width: '80px', height: '33px', padding: '0', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box', borderColor: 'var(--color-denuvo, #e63946)', color: 'var(--color-denuvo, #e63946)' }}
            onClick={handleClearLogs}
            title="Clear current log session file(s)"
          >
            Clear
          </button>
        </div>
      </div>

      <div className="log-view-terminal-wrap">
        <button
          type="button"
          className="log-copy-btn"
          onClick={handleDownloadSource}
          title="Download the current log document (same naming as the .zip)"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path d="M12 3v12m0 0l-4-4m4 4l4-4" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            <path d="M4 21h16" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
          </svg>
        </button>
        <div className="log-view-terminal" ref={containerRef} onScroll={handleScroll}>
          {filteredLines.length > 0 ? (
            filteredLines.map((line, idx) => (
              <div
                key={lineKey(line, idx, snapshotRef.current)}
                className={getLineClass(line)}
              >
                {line}
              </div>
            ))
          ) : (
            <div className="log-empty-state">
              {lines.length === 0
                ? 'No session logs recorded yet.'
                : debouncedFilter
                  ? `No log entries match "${debouncedFilter}" at level ${logLevel.toUpperCase()}.`
                  : `No log entries at level ${logLevel.toUpperCase()}.`}
            </div>
          )}
        </div>
      </div>
    </div>
  );
});
