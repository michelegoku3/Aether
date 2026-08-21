import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emptyStatus, StatusMessage } from '../types/ui';
import { useModalDismiss } from '../hooks/useModalDismiss';

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

const DROP_ZONE_HINT = 'Click to browse for crack file(s)';

const basename = (path: string): string => path.split(/[\\/]/).pop() ?? path;

export const CrackModal = ({ game, onClose }: CrackModalProps) => {
  // Multiple files are supported: dropped files append to the list and are
  // shown stacked, one per line.
  const [selectedFiles, setSelectedFiles] = useState<string[]>([]);
  const [isApplying, setIsApplying] = useState(false);
  const [vnPatchMode, setVnPatchMode] = useState(false);
  const [status, setStatus] = useState<StatusMessage>(emptyStatus());

  const showStatus = (text: string, type: StatusMessage['type']) =>
    setStatus({ text, type });

  const addFiles = (paths: string[]) => {
    setSelectedFiles((prev) => [...prev, ...paths]);
    setStatus(emptyStatus());
  };

  // Click on the drop zone → native OS file picker.
  const openFilePicker = async () => {
    try {
      const paths: string[] = await invoke('pick_crack_files', {
        appId: Number(game.appId),
      });
      if (paths.length > 0) addFiles(paths);
    } catch (err: any) {
      showStatus(`Failed to open file picker: ${err}`, 'error');
    }
  };

  // ESC + click fuori chiudono il popup (rispettando un'operazione in corso).
  useModalDismiss(onClose, isApplying);

  const clearFiles = () => {
    setSelectedFiles([]);
    setVnPatchMode(false);
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
        vnPatchMode: vnPatchMode,
      });
      showStatus(message, 'success');
    } catch (err: any) {
      showStatus(`${err}`, 'error');
    } finally {
      setIsApplying(false);
    }
  };

  const fileNames = selectedFiles.map(basename);


  return (
    <div className="modal-overlay" onClick={isApplying ? undefined : onClose}>
      <div className="modal-container crack-modal-container" onClick={(e) => e.stopPropagation()}>
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
            aria-label="Select crack file(s)"
            onClick={() => {
              if (!isApplying) openFilePicker();
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !isApplying) openFilePicker();
            }}
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

          <div className="crack-options-group">
            <label className="crack-achievement-row" title="Achievement compatibility is not available yet">
              <span className="crack-achievement-text">
                Make the crack achievement compatible (it could break the crack)
              </span>
              <span className="crack-checkbox-label">
                <input
                  type="checkbox"
                  className="crack-checkbox-input"
                  checked={false}
                  disabled={true}
                />
                <span className="crack-checkbox-box"></span>
              </span>
            </label>

            <label
              className="crack-achievement-row"
              title="When ON, Visual Novel self-extracting .exe patches are staged and extracted as archives (.exe.zip) so their contents can be applied automatically"
            >
              <span className="crack-achievement-text">
                Make Apply Crack work for VNs .exe patches
              </span>
              <span className="crack-checkbox-label">
                <input
                  type="checkbox"
                  className="crack-checkbox-input"
                  checked={vnPatchMode}
                  onChange={(e) => setVnPatchMode(e.target.checked)}
                />
                <span className="crack-checkbox-box"></span>
              </span>
            </label>
          </div>

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
