import { useEffect, useRef, useState } from 'react';
import type { DragEvent as ReactDragEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { emptyStatus, StatusMessage } from '../types/ui';
import { AntivirusExclusionModal } from './AntivirusExclusionModal';
import { useModalDismiss } from '../hooks/useModalDismiss';

// The game local content is being installed for. `name` and `appId` are enough
// for the popup; all heavy work happens in the Rust backend.
export interface LocalTargetGame {
  name: string;
  appId: string;
}

interface LocalDownloadModalProps {
  game: LocalTargetGame;
  onClose: () => void;
  // Called after a successful install so the parent can close the download
  // modal and refresh usage statistics.
  onInstalled?: () => void;
}

const DROP_ZONE_HINT = 'Click to browse or drag & drop game archive(s) or file(s) here';

// Maximum number of file rows shown in the drop zone; extra files collapse
// into a single ellipsis row so the popup never grows too tall.
const MAX_VISIBLE_FILES = 5;

const basename = (path: string): string => path.split(/[\\/]/).pop() ?? path;

export const LocalDownloadModal = ({ game, onClose, onInstalled }: LocalDownloadModalProps) => {
  // Multiple files are supported: dropped files append to the list and are
  // shown stacked, one per line.
  const [selectedFiles, setSelectedFiles] = useState<string[]>([]);
  const [isDragOver, setIsDragOver] = useState(false);
  const [isInstalling, setIsInstalling] = useState(false);
  const [showAntivirus, setShowAntivirus] = useState(false);
  const [status, setStatus] = useState<StatusMessage>(emptyStatus());

  const showStatus = (text: string, type: StatusMessage['type']) =>
    setStatus({ text, type });

  const addFiles = (paths: string[]) => {
    setSelectedFiles((prev) => [...prev, ...paths]);
    setStatus(emptyStatus());
  };

  // Keep the latest addFiles in a ref so the drag-drop subscriptions below can
  // be set up once (mount) without re-subscribing on every state change.
  const addFilesRef = useRef(addFiles);
  addFilesRef.current = addFiles;

  // Click on the drop zone → native OS file picker (backend opens the dialog).
  const openFilePicker = async () => {
    try {
      const paths: string[] = await invoke('pick_local_files', {
        appId: Number(game.appId),
      });
      if (paths && paths.length > 0) addFiles(paths);
    } catch (err: any) {
      showStatus(`Failed to open file picker: ${err}`, 'error');
    }
  };

  // Drag & drop of OS files — same dual-path strategy as CrackModal:
  //   1. Native: getCurrentWebviewWindow().onDragDropEvent(...) yields real
  //      filesystem paths inside the Tauri WebView.
  //   2. Browser fallback: element-level onDragOver/onDrop, used only outside
  //      a Tauri WebView and guarded so it never double-processes.
  useEffect(() => {
    // Allow a valid drop cursor across the whole window.
    const preventDragDefault = (event: DragEvent) => event.preventDefault();
    window.addEventListener('dragover', preventDragDefault);

    let unlisten: (() => void) | null = null;
    getCurrentWebviewWindow()
      .onDragDropEvent((event) => {
        const type = event.payload.type;
        if (type === 'enter') {
          setIsDragOver(true);
        } else if (type === 'leave') {
          setIsDragOver(false);
        } else if (type === 'drop') {
          setIsDragOver(false);
          addFilesRef.current(event.payload.paths);
        }
      })
      .then((unlistenFn) => {
        unlisten = unlistenFn;
      });

    return () => {
      unlisten?.();
      window.removeEventListener('dragover', preventDragDefault);
    };
  }, []);

  // ESC + click fuori chiudono il popup (rispettando un'installazione in corso).
  useModalDismiss(onClose, isInstalling);

  const clearFiles = () => {
    setSelectedFiles([]);
    setStatus(emptyStatus());
  };

  // Antivirus gate at the START of a new installation (not at Apply Crack).
  // If the user already granted the exclusion (settings.json value), nothing
  // is shown and the install proceeds directly.
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
    showStatus('Installing local content into Steam...', 'info');
    try {
      const message: string = await invoke('install_local_game', {
        appId: Number(game.appId),
        appName: game.name,
        localFiles: selectedFiles,
      });
      const hasBuildWarning = message.includes('Warning:');
      showStatus(message, hasBuildWarning ? 'info' : 'success');

      // Keep advisory build-mismatch warnings visible so the user can read
      // them. Ordinary successful installs retain the normal auto-close.
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

  // Browser fallback handler (only acts outside a Tauri WebView).
  const handleBrowserDrop = (event: ReactDragEvent) => {
    event.preventDefault();
    if ('__TAURI_INTERNALS__' in window) return; // native handler owns it
    const files = Array.from(event.dataTransfer.files);
    if (files.length > 0) {
      addFiles(files.map((file) => file.name));
    }
  };

  return (
    <div className="modal-overlay" onClick={isInstalling ? undefined : onClose}>
      <div className="modal-container crack-modal-container" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">
            Local Install for: <strong style={{ color: '#ffffff' }}>{game.name}</strong> ({game.appId})
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
            className={`crack-drop-zone ${isDragOver ? 'drag-over' : ''} ${
              fileNames.length > 0 ? 'has-file' : ''
            }`}
            role="button"
            tabIndex={0}
            aria-label="Select game archive(s) or file(s)"
            onClick={() => {
              if (!isInstalling) openFilePicker();
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !isInstalling) openFilePicker();
            }}
            onDragOver={(event) => event.preventDefault()}
            onDrop={handleBrowserDrop}
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
                    … (+{hiddenFileCount} more file{hiddenFileCount === 1 ? '' : 's'})
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
              {isInstalling ? 'Installing...' : 'Install into Steam'}
            </button>
            <button
              className="panel-btn crack-btn"
              onClick={clearFiles}
              disabled={isInstalling || selectedFiles.length === 0}
            >
              Clean File
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
