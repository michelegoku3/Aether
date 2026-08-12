import { useEffect, useState, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';

export const LogView = () => {
  const [lines, setLines] = useState<string[]>([]);
  const [filterQuery, setFilterQuery] = useState('');
  const [logLevel, setLogLevel] = useState('trace');
  const [logSource, setLogSource] = useState<'desk' | 'dll' | 'both'>('desk');
  const [exportStatus, setExportStatus] = useState('');
  const containerRef = useRef<HTMLDivElement | null>(null);
  const isAtBottomRef = useRef(true);

  useEffect(() => {
    invoke('get_settings')
      .then((s: any) => {
        if (s && s.log_level) {
          setLogLevel(s.log_level.toLowerCase());
        }
      })
      .catch(() => {});
  }, []);

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
  }, [logSource]);

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
    return !filterQuery.trim() || line.toLowerCase().includes(filterQuery.toLowerCase());
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

        <div style={{ display: 'flex', alignItems: 'center', gap: '12px', flexWrap: 'wrap' }}>
          <select
            className="settings-select"
            value={logSource}
            style={{ width: '105px', height: '33px', padding: '0 8px', boxSizing: 'border-box' }}
            onChange={(e) => setLogSource(e.target.value as 'desk' | 'dll' | 'both')}
            title="Select log source to view"
          >
            <option value="desk">Desk</option>
            <option value="dll">DLL</option>
            <option value="both">Desk &amp; DLL</option>
          </select>

          <select
            className="settings-select"
            value={logLevel}
            style={{ width: '90px', height: '33px', padding: '0 8px', boxSizing: 'border-box' }}
            onChange={async (e) => {
              const next = e.target.value;
              setLogLevel(next);
              try {
                await invoke('set_session_log_level', { level: next });
              } catch (err) {
                console.warn('Failed to set session log level:', err);
              }
            }}
            title="Set logging level for Desk &amp; DLL"
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
            title="Export AetherDesk & AetherDLL logs as .zip in Downloads folder"
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
