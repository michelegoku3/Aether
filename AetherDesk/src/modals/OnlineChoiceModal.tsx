import type { LibraryActionGame } from './LibraryGameActionsModal';

interface OnlineChoiceModalProps {
  game: LibraryActionGame;
  aetherEnabled: boolean;
  uco2Enabled: boolean;
  busy: boolean;
  onToggleAether: () => void;
  onEnableUco2: () => void;
  onDisableUco2: () => void;
  onClose: () => void;
}

const styles = {
  header: { display: 'flex' as const, justifyContent: 'space-between' as const, alignItems: 'center' as const, padding: '14px 20px', borderBottom: '1px solid #232329' },
  headerTitle: { margin: 0, fontSize: '16px', fontWeight: 700 },
  close: { background: 'none', border: 'none', color: '#999', fontSize: '20px', cursor: 'pointer', lineHeight: 1 },
  body: { padding: '18px 20px' },
  grid: { display: 'grid' as const, gridTemplateColumns: '1fr 1fr', gap: '14px' },
  option: {
    display: 'flex' as const,
    flexDirection: 'column' as const,
    alignItems: 'center' as const,
    justifyContent: 'center' as const,
    gap: '8px',
    padding: '22px 14px',
    borderRadius: '8px',
    background: '#1b1b1f',
    border: '1px solid #2c2c31',
    color: '#eee',
    fontSize: '14px',
    fontWeight: 700,
    cursor: 'pointer',
  },
  optionDesc: { fontSize: '11px', fontWeight: 400, opacity: 0.65, textAlign: 'center' as const, lineHeight: 1.4 },
  optionDisabled: { opacity: 0.4, cursor: 'not-allowed' },
};

/**
 * Popup di scelta per l'online di un gioco: Enable/Disable Aether (aggiunge
 * `-onlinefix` alle LaunchOptions di Steam) oppure Enable/Disable UCO2
 * (apre il pannello di configurazione UCOnline2). I due sono mutuamente
 * esclusivi: quando uno è attivo, l'altro diventa non cliccabile.
 */
export const OnlineChoiceModal = ({
  game,
  aetherEnabled,
  uco2Enabled,
  busy,
  onToggleAether,
  onEnableUco2,
  onDisableUco2,
  onClose,
}: OnlineChoiceModalProps) => {
  const aetherDisabled = busy || (uco2Enabled && !aetherEnabled);
  const uco2Disabled = busy || (aetherEnabled && !uco2Enabled);

  return (
    <div className="modal-overlay" onClick={busy ? undefined : onClose}>
      <div
        className="modal-container"
        style={{ width: 480, maxHeight: '86vh' }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={styles.header}>
          <h3 style={styles.headerTitle}>Online: {game.name} ({game.appId})</h3>
          <button type="button" style={styles.close} onClick={onClose} disabled={busy} aria-label="Close">×</button>
        </div>

        <div style={styles.body}>
          <div style={styles.grid}>
            <button
              type="button"
              className="modal-btn"
              style={{ ...styles.option, ...(aetherDisabled ? styles.optionDisabled : {}) }}
              onClick={onToggleAether}
              disabled={aetherDisabled}
              title={uco2Enabled && !aetherEnabled ? 'Disable UCO2 first' : undefined}
            >
              <span>{aetherEnabled ? 'Disable Aether' : 'Enable Aether'}</span>
              <span style={styles.optionDesc}>Adds the -onlinefix Steam launch option</span>
            </button>

            <button
              type="button"
              className="modal-btn"
              style={{ ...styles.option, ...(uco2Disabled ? styles.optionDisabled : {}) }}
              onClick={uco2Enabled ? onDisableUco2 : onEnableUco2}
              disabled={uco2Disabled}
              title={aetherEnabled && !uco2Enabled ? 'Disable Aether first' : undefined}
            >
              <span>{uco2Enabled ? 'Disable UCO2' : 'Enable UCO2'}</span>
              <span style={styles.optionDesc}>Opens the UCOnline2 setup panel</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
