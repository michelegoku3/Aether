import { useEffect, useState, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';

export const LogView = () => {
  const [lines, setLines] = useState<string[]>([]);
  const [filterQuery, setFilterQuery] = useState('');
  const [levelFilter, setLevelFilter] = useState<string>('ALL');
  const [autoScroll, setAutoScroll] = useState(true);
  const [isLoading, setIsLoading] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);

  const fetchLogs = async () => {
    setIsLoading(true);
    try {
      const recent: string[] = await invoke('get_recent_log_lines', { tailLines: 400 });
      setLines(recent || []);
    } catch (err) {
      console.warn('Failed to fetch logs:', err);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchLogs();
    const interval = setInterval(fetchLogs, 2500);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    if (autoScroll && containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [lines, autoScroll]);

  const handleClearLogs = async () => {
    try {
      await invoke('clear_session_log');
      await fetchLogs();
    } catch (err) {
      console.warn('Failed to clear logs:', err);
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
      <div className="log-view-header">
        <div className="log-header-left">
          <h1 className="home-title" style={{ margin: 0 }}>LOGS</h1>
          <span className="log-live-badge">
            <span className="log-live-dot"></span>
            Live Monitoring — desk.log
          </span>
        </div>

        <div className="log-header-controls">
          <div className="home-search-wrapper" style={{ width: '220px' }}>
            <input
              type="text"
              placeholder="Filter logs by keyword..."
              value={filterQuery}
              onChange={(e) => setFilterQuery(e.target.value)}
              className="store-search-input"
              style={{ padding: '6px 28px 6px 12px', fontSize: '12.5px' }}
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

          <label
            className="version-switch"
            title="Auto-scroll to latest entries"
            style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }}
          >
            <span style={{ fontSize: '12px', color: '#a0a0b0', userSelect: 'none' }}>Auto-scroll</span>
            <input
              type="checkbox"
              checked={autoScroll}
              onChange={(e) => setAutoScroll(e.target.checked)}
            />
            <span className="crack-checkbox-box" style={{ position: 'relative' }}></span>
          </label>

          <button
            type="button"
            className="settings-small-btn"
            onClick={fetchLogs}
            disabled={isLoading}
            title="Refresh logs now"
          >
            Refresh
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

      <div className="log-view-terminal" ref={containerRef}>
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
