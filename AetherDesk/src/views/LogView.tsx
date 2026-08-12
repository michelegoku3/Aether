import { useEffect, useState, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';

export const LogView = () => {
  const [lines, setLines] = useState<string[]>([]);
  const [filterQuery, setFilterQuery] = useState('');
  const [levelFilter, setLevelFilter] = useState<string>('ALL');
  const [exportStatus, setExportStatus] = useState('');
  const containerRef = useRef<HTMLDivElement | null>(null);
  const isAtBottomRef = useRef(true);

  const fetchLogs = async () => {
    try {
      const recent: string[] = await invoke('get_recent_log_lines', { tailLines: 500 });
      setLines(recent || []);
    } catch (err) {
      console.warn('Failed to fetch logs:', err);
    }
  };

  useEffect(() => {
    fetchLogs();
    const interval = setInterval(fetchLogs, 2500);
    return () => clearInterval(interval);
  }, []);

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
      await invoke('clear_session_log');
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
    const queryMatch = !filterQuery.trim() || line.toLowerCase().includes(filterQuery.toLowerCase());
    if (!queryMatch) return false;

    if (levelFilter === 'ALL') return true;
    if (levelFilter === 'INFO') return line.includes('[INFO ]');
    if (levelFilter === 'WARN') return line.includes('[WARN ]');
    if (levelFilter === 'ERROR') return line.includes('[ERROR]');
    if (levelFilter === 'DEBUG') return line.includes('[DEBUG]');
    return true;
  });

  const getLineClass = (line: string) => {
    if (line.includes('[ERROR]')) return 'log-line error';
    if (line.includes('[WARN ]')) return 'log-line warn';
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
          Real-time terminal console monitoring desk.log for session diagnostics and lifecycle events.
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

        <div style={{ display: 'flex', alignItems: 'center', gap: '16px', flexWrap: 'wrap' }}>
          <div className="log-level-pills">
            {(['ALL', 'INFO', 'WARN', 'ERROR'] as const).map((lvl) => (
              <button
                key={lvl}
                type="button"
                className={`log-pill-btn ${levelFilter === lvl ? 'active' : ''}`}
                onClick={() => setLevelFilter(lvl)}
              >
                {lvl}
              </button>
            ))}
          </div>

          <button
            type="button"
            className="settings-small-btn"
            onClick={handleSaveBundle}
            title="Export AetherDesk & AetherDLL logs as .zip in Downloads folder"
          >
            Save
          </button>

          <button
            type="button"
            className="settings-small-btn"
            style={{ borderColor: 'var(--color-denuvo, #e63946)', color: 'var(--color-denuvo, #e63946)' }}
            onClick={handleClearLogs}
            title="Clear current desk.log session file"
          >
            Clear
          </button>
        </div>
      </div>

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
              ? 'No session logs recorded yet in desk.log.'
              : 'No log entries match the active filter.'}
          </div>
        )}
      </div>
    </div>
  );
};
