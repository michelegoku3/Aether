import type { StoreGameResult } from '../../hooks/useStoreSearch';
import type { StatusMessage } from '../../types/ui';
import { StatusAlert } from '../../ui/StatusAlert';
import type { StoreDownloadSource } from './useStoreDownload';

interface StoreDownloadModalProps {
  game: StoreGameResult;
  source: StoreDownloadSource;
  status: StatusMessage;
  isDownloading: boolean;
  onSourceChange: (source: StoreDownloadSource) => void;
  onDownloadLatest: () => void;
  onDownloadSpecific: () => void;
  onClose: () => void;
}

export function StoreDownloadModal({
  game,
  source,
  status,
  isDownloading,
  onSourceChange,
  onDownloadLatest,
  onDownloadSpecific,
  onClose,
}: StoreDownloadModalProps) {
  return (
    <div className="modal-overlay" onClick={isDownloading ? undefined : onClose}>
      <div className="modal-container" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">
            Download: <strong style={{ color: '#ffffff' }}>{game.name}</strong> ({game.appId})
          </span>
          <button
            onClick={onClose}
            className="modal-close-btn"
            disabled={isDownloading}
            style={{ opacity: isDownloading ? 0.3 : 1 }}
          >
            &times;
          </button>
        </div>
        <div className="modal-separator"></div>
        <div className="modal-body">
          <StatusAlert status={status} className="settings-alert--compact" />
          <div className="source-box">
            <span className="source-label">Source:</span>
            <div className="source-buttons-row">
              <button
                disabled={isDownloading}
                onClick={() => onSourceChange('hubcap')}
                className={`source-btn ${source === 'hubcap' ? 'active' : ''}`}
              >
                Hubcap
              </button>
              <button
                disabled={isDownloading}
                onClick={() => onSourceChange('luatools')}
                className={`source-btn ${source === 'luatools' ? 'active' : ''}`}
                title="Uses your signed-in lua.tools account and automatically selects an available source"
              >
                LuaTools
              </button>
              <button
                disabled={isDownloading}
                onClick={() => onSourceChange('ryuu')}
                className={`source-btn ${source === 'ryuu' ? 'active' : ''}`}
              >
                Ryuu
              </button>
              <button disabled className="source-btn source-btn-disabled" title="MOED source is currently unavailable">
                MOED
              </button>
            </div>
          </div>

          <button onClick={onDownloadLatest} className="big-action-btn" disabled={isDownloading}>
            <div className="action-icon">⚡</div>
            <div className="action-info">
              <span className="action-title">Download Latest Version</span>
              <span className="action-desc">
                Downloads the most recent manifest files and decryption keys directly into Steam. This allows Steam to download and install the latest official release.
              </span>
            </div>
          </button>

          <button onClick={onDownloadSpecific} className="big-action-btn" disabled={isDownloading}>
            <div className="action-icon">📦</div>
            <div className="action-info">
              <span className="action-title">Download Specific Version</span>
              <span className="action-desc">
                Allows you to pick and download a specific historical version or downgrade release of the game by pinning custom Steam manifest IDs.
              </span>
            </div>
          </button>
        </div>
      </div>
    </div>
  );
}
