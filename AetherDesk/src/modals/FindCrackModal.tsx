import React from 'react';

type FindCrackResource = 'onlinefix' | 'gcw' | 'csrinru';

interface FindCrackModalProps {
  onClose: () => void;
  onSelect: (site: FindCrackResource) => void;
}

export const FindCrackModal: React.FC<FindCrackModalProps> = ({ onClose, onSelect }) => {
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-container" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">
            Find Crack — <strong style={{ color: '#ffffff' }}>Select source</strong>
          </span>
          <button onClick={onClose} className="modal-close-btn">
            &times;
          </button>
        </div>

        <div className="modal-separator"></div>

        <div className="modal-body">
          <p className="settings-desc" style={{ textAlign: 'center', marginBottom: '4px' }}>
            Choose a crack source to open in your browser.
          </p>

          <div className="home-action-grid" style={{ gridTemplateColumns: 'repeat(2, 172px)', justifyContent: 'center' }}>
            <button className="game-action-btn" onClick={() => onSelect('onlinefix')}>
              OnlineFix
            </button>
            <button className="game-action-btn" onClick={() => onSelect('gcw')}>
              GCW
            </button>
            <button className="game-action-btn" onClick={() => onSelect('csrinru')}>
              CSRINRU
            </button>
            <button className="game-action-btn" disabled title="XATAB is not available yet">
              XATAB
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
