import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emptyStatus, StatusMessage } from '../types/ui';
import { useModalDismiss } from '../hooks/useModalDismiss';

interface AntivirusExclusionModalProps {
  /**
   * Called when the modal is dismissed. `confirmed` is true when the user
   * explicitly added the exclusion (automatically or manually) and false when
   * they closed via the X button — letting the caller decide whether to
   * proceed with the risky operation anyway.
   */
  onDone: (confirmed: boolean) => void;
}

/**
 * Modal that prompts the user to add AetherDesk and all Steam library folders
 * to Windows Defender exclusions. Crack files are written into Steam library
 * folders, so those folders must be excluded or Defender will quarantine them.
 *
 * Shown in two contexts:
 *   - On first launch after install/update (when `antivirus_exclusion_done` is false).
 *   - When the user clicks "Apply Crack" in Home (if never confirmed).
 *
 * Once the user confirms (Add exclusion or I added it manually), the flag is
 * persisted and the modal never appears again. Closing via X dismisses without
 * persisting, so the prompt will reappear next time.
 */
export const AntivirusExclusionModal = ({ onDone }: AntivirusExclusionModalProps) => {
  const [isApplying, setIsApplying] = useState(false);
  const [status, setStatus] = useState<StatusMessage>(emptyStatus());

  // ESC / click fuori chiudono senza confermare (come la X): onDone(false)
  // non persiste il flag, quindi il popup riapparirà alla prossima occasione.
  const dismiss = () => onDone(false);
  useModalDismiss(dismiss, isApplying);

  const handleApply = async () => {
    setIsApplying(true);
    setStatus({ text: 'Adding exclusions for AetherDesk and Steam libraries...', type: 'info' });
    try {
      const message: string = await invoke('apply_antivirus_exclusion');
      setStatus({ text: message, type: 'success' });
      onDone(true);
    } catch (err: any) {
      setStatus({ text: `Could not add the exclusion automatically: ${err}`, type: 'error' });
    } finally {
      setIsApplying(false);
    }
  };

  const handleDoneManually = async () => {
    try {
      await invoke('acknowledge_antivirus_exclusion');
    } catch {
      // best-effort persistence; the caller still treats this as confirmed
    }
    onDone(true);
  };

  return (
    <div className="modal-overlay" onClick={isApplying ? undefined : dismiss}>
      <div className="modal-container av-modal-container" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">
            Windows Defender Exclusion
          </span>
          <button
            className="modal-close-btn"
            disabled={isApplying}
            onClick={() => {
              if (!isApplying) onDone(false);
            }}
          >
            &times;
          </button>
        </div>

        <div className="modal-separator"></div>

        <div className="modal-body">
          <p className="av-modal-text">
            Crack files applied by AetherDesk are placed inside your Steam library
            folders and can be flagged by Windows Defender as false positives —
            which may silently delete them or break your games. To prevent this,
            AetherDesk and all your Steam library folders should be added to
            Defender's exclusion list.
          </p>

          {status.text && (
            <div className={`settings-alert ${status.type} settings-alert--compact`}>
              {status.text}
            </div>
          )}

          <div className="av-modal-actions">
            <button
              className="panel-btn"
              onClick={handleApply}
              disabled={isApplying}
            >
              {isApplying ? 'Adding...' : 'Add exclusion'}
            </button>

            <button
              className="av-link-btn"
              onClick={handleDoneManually}
              disabled={isApplying}
            >
              I added it manually
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
