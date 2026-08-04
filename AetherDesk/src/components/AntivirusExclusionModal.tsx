import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emptyStatus, StatusMessage } from '../types/ui';

interface AntivirusExclusionModalProps {
  onDone: () => void; // called once the user has handled the exclusion
}

export const AntivirusExclusionModal = ({ onDone }: AntivirusExclusionModalProps) => {
  const [isApplying, setIsApplying] = useState(false);
  const [status, setStatus] = useState<StatusMessage>(emptyStatus());

  // ESC closes the modal without marking done (it will ask again next run).
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !isApplying) {
        onDone();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isApplying, onDone]);

  const handleApply = async () => {
    setIsApplying(true);
    setStatus({ text: 'Adding Windows Defender exclusion...', type: 'info' });
    try {
      const message: string = await invoke('apply_antivirus_exclusion');
      setStatus({ text: message, type: 'success' });
      onDone();
    } catch (err: any) {
      setStatus({ text: `Could not add the exclusion automatically: ${err}`, type: 'error' });
    } finally {
      setIsApplying(false);
    }
  };

  const handleOpenSecurity = () => {
    invoke('open_windows_security').catch(() => {});
  };

  const handleDoneManually = async () => {
    try {
      await invoke('acknowledge_antivirus_exclusion');
    } catch {
      // best-effort
    }
    onDone();
  };

  return (
    <div className="modal-overlay">
      <div className="modal-container av-modal-container">
        <div className="modal-header">
          <span className="modal-title">
            Windows Defender Exclusion
          </span>
          <button
            className="modal-close-btn"
            disabled={isApplying}
            onClick={() => {
              if (!isApplying) onDone();
            }}
          >
            &times;
          </button>
        </div>

        <div className="modal-separator"></div>

        <div className="modal-body">
          <p className="av-modal-text">
            AetherDesk and the crack files it applies can be flagged by Windows
            Defender as false positives, which may block or remove them.
            Add the AetherDesk folder to Windows Defender exclusions to prevent
            this.
          </p>

          {status.text && (
            <div className={`settings-alert ${status.type}`} style={{ padding: '10px 15px', fontSize: '12px' }}>
              {status.text}
            </div>
          )}

          <div className="av-modal-actions">
            <button className="panel-btn" onClick={handleApply} disabled={isApplying}>
              {isApplying ? 'Adding...' : 'Add exclusion automatically'}
            </button>
            <button className="panel-btn" onClick={handleOpenSecurity} disabled={isApplying}>
              Open Windows Security
            </button>
          </div>

          <div className="av-modal-links">
            <button className="av-link-btn" onClick={handleDoneManually} disabled={isApplying}>
              I added it manually
            </button>
            <button className="av-link-btn" onClick={() => onDone()} disabled={isApplying}>
              Not now
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
