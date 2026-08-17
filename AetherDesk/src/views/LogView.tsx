import { useEffect, useState, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';

const LEVEL_RANK: Record<string, number> = {
  trace: 0,
  debug: 1,
  info: 2,
  warn: 3,
  error: 4,
  off: 5,
};

const lineRank = (line: string) => {
  if (line.includes('[ERROR]')) return 4;
  if (line.includes('[WARN ]') || line.includes('[WARN]')) return 3;
  if (line.includes('[INFO ]') || line.includes('[INFO]')) return 2;
  if (line.includes('[DEBUG]')) return 1;
  if (line.includes('[TRACE]')) return 0;
  return 2;
};

export const LogView = () => {
  const [lines, setLines] = useState<string[]>([]);
  const [filterQuery, setFilterQuery] = useState('');
  const [logLevel, setLogLevel] = useState('trace');
  const [logSource, setLogSource] = useState<'desk' | 'dll' | 'uco2' | 'both'>('desk');
  const [exportStatus, setExportStatus] = useState('');
  const containerRef = useRef<HTMLDivElement | null>(null);
  const isAtBottomRef = useRef(true);

  const fetchLogs = async () => {
    try {
      const recent: string[] = await invoke('get_recent_log_lines', {
        tailLines: 500,
        source: logSource,
      });
      setLines(recent || []);
    } catch (err) {
      console.warn('Failed to fetch logs:', err);
    }
  };

  useEffect(() => {
    fetchLogs();
    const interval = setInterval(fetchLogs, 2500);
    return () => clearInterval(interval);
  }, [logSource]);

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

  const filteredLines = lines.filter((line) => {
    if (logLevel === 'off') return false;
    const minRank = LEVEL_RANK[logLevel] ?? 0;
    if (lineRank(line) < minRank) return false;
    return !filterQuery.trim() || line.toLowerCase().includes(filterQuery.toLowerCase());
  });

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
              <div key={idx} className={getLineClass(line)}>
                {line}
              </div>
            ))
          ) : (
            <div className="log-empty-state">
              {lines.length === 0
                ? 'No session logs recorded yet.'
                : 'No log entries match the active filter.'}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
