import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emptyStatus, StatusMessage } from '../types/ui';
import { AntivirusExclusionModal } from './AntivirusExclusionModal';
import { useModalDismiss } from '../hooks/useModalDismiss';
import { useLibraryGames } from '../hooks/useLibraryGames';

// Optional target game for backwards compatibility. When omitted, the modal
// operates in Bulk Mode (recursive search for any .lua and .manifest).
export interface LocalTargetGame {
  name: string;
  appId: string;
}

interface LocalDownloadModalProps {
  game?: LocalTargetGame | null;
  onClose: () => void;
  // Called after a successful install so the parent can refresh statistics.
  onInstalled?: () => void;
}

const DROP_ZONE_HINT = 'Click to browse for .lua, .manifest, folders, or archives (.zip, .7z, .rar)';

// Maximum number of file rows shown in the drop zone; extra files collapse
// into a single ellipsis row so the popup never grows too tall.
const MAX_VISIBLE_FILES = 5;

const basename = (path: string): string => path.split(/[\\/]/).pop() ?? path;

export const LocalDownloadModal = ({ game, onClose, onInstalled }: LocalDownloadModalProps) => {
  const [selectedFiles, setSelectedFiles] = useState<string[]>([]);
  const [isInstalling, setIsInstalling] = useState(false);
  const [showAntivirus, setShowAntivirus] = useState(false);
  const [status, setStatus] = useState<StatusMessage>(emptyStatus());
  const { loadInstalledGames } = useLibraryGames();

  const showStatus = (text: string, type: StatusMessage['type']) =>
    setStatus({ text, type });

  const addFiles = (paths: string[]) => {
    setSelectedFiles((prev) => {
      const set = new Set(prev);
      for (const p of paths) {
        if (p && p.trim()) set.add(p);
      }
      return Array.from(set);
    });
    setStatus(emptyStatus());
  };

  // Click on the drop zone → native OS file/folder picker.
  const openFilePicker = async () => {
    try {
      const paths: string[] = await invoke('pick_local_files', {
        appId: game ? Number(game.appId) : undefined,
      });
      if (paths && paths.length > 0) addFiles(paths);
    } catch (err: any) {
      showStatus(`Failed to open file picker: ${err}`, 'error');
    }
  };

  // ESC + click fuori chiudono il popup (rispettando un'installazione in corso).
  useModalDismiss(onClose, isInstalling);

  const clearFiles = () => {
    setSelectedFiles([]);
    setStatus(emptyStatus());
  };

  const handleInstall = async () => {
    if (selectedFiles.length === 0) return;

    try {
      const done: boolean = await invoke('get_antivirus_exclusion_done');
      if (!done) {
        setShowAntivirus(true);
        return;
      }
    } catch {
      // On error, do not block the install.
    }
    await doInstall();
  };

  const doInstall = async () => {
    setIsInstalling(true);
    showStatus('Scanning and importing .lua and .manifest files into Steam...', 'info');
    try {
      let message: string;
      if (game) {
        message = await invoke('install_local_game', {
          appId: Number(game.appId),
          appName: game.name,
          localFiles: selectedFiles,
        });
      } else {
        message = await invoke('install_bulk_local', {
          localFiles: selectedFiles,
        });
      }

      // The backend watcher/revision channel covers independent filesystem
      // changes. This direct UI path starts the shared canonical scan at once.
      loadInstalledGames();
      const hasBuildWarning = message.includes('Warning:');
      showStatus(message, hasBuildWarning ? 'info' : 'success');

      if (!hasBuildWarning) {
        setTimeout(() => {
          onInstalled?.();
        }, 3000);
      }
    } catch (err: any) {
      showStatus(`${err}`, 'error');
    } finally {
      setIsInstalling(false);
    }
  };

  const fileNames = selectedFiles.map(basename);
  const visibleFileNames = fileNames.slice(0, MAX_VISIBLE_FILES);
  const hiddenFileCount = fileNames.length - visibleFileNames.length;

  const modalTitle = game
    ? `Local Install for: ${game.name} (${game.appId})`
    : 'Local Import (Bulk Lua & Manifests)';

  return (
    <div className="modal-overlay" onClick={isInstalling ? undefined : onClose}>
      <div className="modal-container crack-modal-container" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">
            {game ? (
              <>
                Local Install for: <strong style={{ color: '#ffffff' }}>{game.name}</strong> ({game.appId})
              </>
            ) : (
              <strong style={{ color: '#ffffff' }}>{modalTitle}</strong>
            )}
          </span>
          <button
            onClick={() => {
              if (!isInstalling) onClose();
            }}
            className="modal-close-btn"
            disabled={isInstalling}
            style={{ opacity: isInstalling ? 0.3 : 1 }}
          >
            &times;
          </button>
        </div>

        <div className="modal-separator"></div>

        <div className="modal-body">
          {status.text && (
            <div className={`settings-alert ${status.type} settings-alert--compact`}>
              {status.text}
            </div>
          )}

          <div
            className={`crack-drop-zone ${
              fileNames.length > 0 ? 'has-file' : ''
            }`}
            role="button"
            tabIndex={0}
            aria-label="Select files, folders, or archives"
            onClick={() => {
              if (!isInstalling) openFilePicker();
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !isInstalling) openFilePicker();
            }}
          >
            {fileNames.length > 0 ? (
              <div className="crack-drop-files">
                {visibleFileNames.map((name, index) => (
                  <span key={index} className="crack-drop-filename" title={selectedFiles[index]}>
                    {name}
                  </span>
                ))}
                {hiddenFileCount > 0 && (
                  <span
                    className="crack-drop-filename"
                    title={selectedFiles.slice(MAX_VISIBLE_FILES).join('\n')}
                  >
                    … (+{hiddenFileCount} more item{hiddenFileCount === 1 ? '' : 's'})
                  </span>
                )}
              </div>
            ) : (
              <span className="crack-drop-hint">{DROP_ZONE_HINT}</span>
            )}
          </div>

          <div className="crack-actions">
            <button
              className="panel-btn crack-btn"
              onClick={handleInstall}
              disabled={isInstalling || selectedFiles.length === 0}
            >
              {isInstalling ? 'Importing...' : 'Import into Steam'}
            </button>
            <button
              className="panel-btn crack-btn"
              onClick={clearFiles}
              disabled={isInstalling || selectedFiles.length === 0}
            >
              Clean List
            </button>
          </div>
        </div>
      </div>

      {/* Antivirus exclusion prompt: shown at the start of a new installation
          when the user never granted the exclusion (settings.json). After the
          choice, the install continues. */}
      {showAntivirus && (
        <AntivirusExclusionModal
          onDone={() => {
            setShowAntivirus(false);
            void doInstall();
          }}
        />
      )}
    </div>
  );
};
