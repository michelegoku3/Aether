import { useEffect, useRef, useState } from 'react';
import type { DragEvent as ReactDragEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { emptyStatus, StatusMessage } from '../types/ui';

// The game a crack is being applied to. `name` and `appId` are enough for the
// popup; all heavy work happens in the Rust backend.
export interface CrackTargetGame {
  name: string;
  appId: string;
}

interface CrackModalProps {
  game: CrackTargetGame;
  onClose: () => void;
}

const DROP_ZONE_HINT = 'Click to browse or drag & drop crack file(s) here';

const basename = (path: string): string => path.split(/[\\/]/).pop() ?? path;

export const CrackModal = ({ game, onClose }: CrackModalProps) => {
  // Multiple files are supported: dropped files append to the list and are
  // shown stacked, one per line.
  const [selectedFiles, setSelectedFiles] = useState<string[]>([]);
  const [isDragOver, setIsDragOver] = useState(false);
  const [isApplying, setIsApplying] = useState(false);
  // Achievement compatibility is an opt-in, off by default ("disattivata di
  // default") because it may break the crack. Currently UI-only; it will be
  // wired into the backend when the apply logic is finalized.
  const [achievementCompatible, setAchievementCompatible] = useState(false);
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
      const paths: string[] = await invoke('pick_crack_files', {
        appId: Number(game.appId),
      });
      if (paths && paths.length > 0) addFiles(paths);
    } catch (err: any) {
      showStatus(`Failed to open file picker: ${err}`, 'error');
    }
  };

  // Drag & drop of OS files.
  //
  // On Windows/WebView2 this is a known, partially-unresolved area in Tauri:
  //   * With dragDropEnabled = true (our config) Tauri registers an OS-level
  //     drop listener that can "hijack" the window, sometimes showing a 🚫
  //     cursor and not delivering paths.
  //   * The browser onDragOver/onDrop DOM events do NOT fire for OS file drops
  //     inside a Tauri WebView.
  // We therefore use BOTH paths defensively:
  //   1. Native: getCurrentWebviewWindow().onDragDropEvent(...) — the primary
  //      production path that yields real filesystem paths.
  //   2. Browser fallback: element-level onDragOver/onDrop reading
  //      e.dataTransfer.files — only used when running outside a Tauri WebView
  //      (e.g. plain browser), guarded so it never double-processes.
  // A window-level dragover preventDefault is kept so the drop cursor is shown
  // as valid ("copy") instead of the OS "blocked" cursor.
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

  // ESC closes the popup (respecting an in-flight apply operation).
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !isApplying) {
        onClose();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isApplying, onClose]);

  const clearFiles = () => {
    setSelectedFiles([]);
    setStatus(emptyStatus());
  };

  const handleApply = async () => {
    if (selectedFiles.length === 0) return;

    setIsApplying(true);
    showStatus('Applying crack...', 'info');
    try {
      const message: string = await invoke('apply_crack', {
        appId: Number(game.appId),
        crackFiles: selectedFiles,
      });
      showStatus(message, 'success');
    } catch (err: any) {
      showStatus(`Failed to apply crack: ${err}`, 'error');
    } finally {
      setIsApplying(false);
    }
  };

  const fileNames = selectedFiles.map(basename);

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
    <div className="modal-overlay">
      <div className="modal-container crack-modal-container">
        <div className="modal-header">
          <span className="modal-title">
            Apply Crack for: <strong style={{ color: '#ffffff' }}>{game.name}</strong> ({game.appId})
          </span>
          <button
            onClick={() => {
              if (!isApplying) onClose();
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
            <div
              className={`settings-alert ${status.type}`}
              style={{ padding: '10px 15px', fontSize: '12px' }}
            >
              {status.text}
            </div>
          )}

          <div
            className={`crack-drop-zone ${isDragOver ? 'drag-over' : ''} ${
              fileNames.length > 0 ? 'has-file' : ''
            }`}
            role="button"
            tabIndex={0}
            aria-label="Select crack file(s)"
            onClick={() => {
              if (!isApplying) openFilePicker();
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !isApplying) openFilePicker();
            }}
            onDragOver={(event) => event.preventDefault()}
            onDrop={handleBrowserDrop}
          >
            {fileNames.length > 0 ? (
              <div className="crack-drop-files">
                {fileNames.map((name, index) => (
                  <span key={index} className="crack-drop-filename" title={selectedFiles[index]}>
                    {name}
                  </span>
                ))}
              </div>
            ) : (
              <span className="crack-drop-hint">{DROP_ZONE_HINT}</span>
            )}
          </div>

          <label className="crack-achievement-row">
            <span className="crack-achievement-text">
              Make the crack achievement compatible (it could break the crack)
            </span>
            {/* Reuse the app's existing toggle (`.version-switch`): dark track in
                the palette, cyan when active. Off by default. */}
            <span className="version-switch">
              <input
                type="checkbox"
                checked={achievementCompatible}
                disabled={isApplying}
                onChange={(event) => setAchievementCompatible(event.target.checked)}
              />
              <span></span>
            </span>
          </label>

          <div className="crack-actions">
            <button
              className="panel-btn crack-btn"
              onClick={handleApply}
              disabled={isApplying || selectedFiles.length === 0}
            >
              {isApplying ? 'Applying...' : 'Apply Crack'}
            </button>
            <button
              className="panel-btn crack-btn"
              onClick={clearFiles}
              disabled={isApplying || selectedFiles.length === 0}
            >
              Clean File
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
