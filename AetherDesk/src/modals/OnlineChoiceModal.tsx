import type { LibraryActionGame } from './LibraryGameActionsModal';
import { useModalDismiss } from '../hooks/useModalDismiss';

// Modalità presenza/online per app, allineata 1:1 con gli array [presence] di
// aethercore.toml (docs/05 §12): none = exclude_apps (hard opt-out),
// showonline = showonline_apps, onlinefix = onlinefix_apps.
export type AppPresenceMode = 'none' | 'showonline' | 'onlinefix';

interface OnlineChoiceModalProps {
  game: LibraryActionGame;
  mode: AppPresenceMode;         // modalità EFFETTIVA corrente (fallback compreso)
  uco2Enabled: boolean;
  busy: boolean;
  onSelectMode: (mode: AppPresenceMode) => void;
  onOpenUco2Panel: () => void;
  onClose: () => void;
}

const styles = {
  header: { display: 'flex' as const, justifyContent: 'space-between' as const, alignItems: 'center' as const, padding: '14px 20px', borderBottom: '1px solid #232329' },
  headerTitle: { margin: 0, fontSize: '16px', fontWeight: 700 },
  close: { background: 'none', border: 'none', color: '#999', fontSize: '20px', cursor: 'pointer', lineHeight: 1 },
  body: { padding: '18px 20px' },
  // 4 opzioni impilate: 1 colonna × 4 righe (None / Show Online / Online
  // Aether / UCO2), full-width, niente griglia 2×2.
  grid: { display: 'grid' as const, gridTemplateColumns: '1fr', gap: '10px' },
  option: {
    display: 'flex' as const,
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    justifyContent: 'flex-start' as const,
    gap: '12px',
    padding: '12px 14px',
    borderRadius: '8px',
    background: '#1b1b1f',
    border: '1px solid #2c2c31',
    color: '#eee',
    fontSize: '13px',
    fontWeight: 700,
    cursor: 'pointer',
    textAlign: 'left' as const,
  },
  optionActive: { borderColor: '#4caf50', boxShadow: 'inset 3px 0 0 #4caf50' },
  optionUco2Active: { borderColor: '#9c6bff', boxShadow: 'inset 3px 0 0 #9c6bff' },
  optionDisabled: { opacity: 0.4, cursor: 'not-allowed' },
  optionTitle: { minWidth: '110px', flexShrink: 0 },
  optionHint: { fontSize: '11px', fontWeight: 400, color: '#9a9aa0', lineHeight: 1.35, flex: 1 },
  badge: { fontSize: '10px', fontWeight: 700, letterSpacing: '0.5px', color: '#4caf50' },
  badgeUco2: { fontSize: '10px', fontWeight: 700, letterSpacing: '0.5px', color: '#9c6bff' },
};

const MODE_CARDS: {
  key: AppPresenceMode;
  title: string;
  hint: string;
}[] = [
  {
    key: 'none',
    title: 'None',
    hint: 'Aether ignores this game: no presence, no online, never any launch argument (hard opt-out).',
  },
  {
    key: 'showonline',
    title: 'Show Online',
    hint: 'Friends see what you play (presence only, no online features). Default for every game.',
  },
  {
    key: 'onlinefix',
    title: 'Online Aether',
    hint: 'Full Spacewar mask + friend presence for OnlineFix sessions. Exclusive with UCO2.',
  },
];

/**
 * Popup scelta online (segmented control): le tre modalità Aether sono
 * mutuamente esclusive (radio); UCO2 è un toggle ortogonale, gestito dal suo
 * pannello — apribile sempre tranne quando Online Aether è attivo (le due
 * masking pipeline si escludono, docs/05 §12). Simmetricamente Online Aether
 * è disabilitato finché UCO2 è deployato. Cliccare la modalità già attiva è
 * un no-op: per "spegnere" si seleziona None.
 */
export const OnlineChoiceModal = ({
  game,
  mode,
  uco2Enabled,
  busy,
  onSelectMode,
  onOpenUco2Panel,
  onClose,
}: OnlineChoiceModalProps) => {
  // ESC chiude il popup (uniforme con gli altri modali); il click fuori è
  // gestito dall'overlay sotto. Entrambi bloccati quando busy.
  useModalDismiss(onClose, busy);

  const onlineFixActive = mode === 'onlinefix';
  const uco2Disabled = busy || onlineFixActive;

  return (
    <div className="modal-overlay" onClick={busy ? undefined : onClose}>
      <div
        className="modal-container"
        style={{ width: 520, maxHeight: '86vh' }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={styles.header}>
          <h3 style={styles.headerTitle}>Online: {game.name} ({game.appId})</h3>
          <button type="button" style={styles.close} onClick={onClose} disabled={busy} aria-label="Close">×</button>
        </div>

        <div style={styles.body}>
          <div style={styles.grid}>
            {MODE_CARDS.map((card) => {
              const isActive = mode === card.key;
              const blocked = busy || (card.key === 'onlinefix' && uco2Enabled);
              const title = card.key === 'onlinefix' && uco2Enabled && !isActive
                ? 'Disable UCO2 first'
                : isActive
                  ? 'Currently active'
                  : `Switch to ${card.title}`;
              return (
                <button
                  key={card.key}
                  type="button"
                  className="modal-btn"
                  style={{
                    ...styles.option,
                    ...(isActive ? styles.optionActive : {}),
                    ...(blocked ? styles.optionDisabled : {}),
                  }}
                  onClick={() => { if (!isActive) onSelectMode(card.key); }}
                  disabled={blocked}
                  title={title}
                >
                  <span style={styles.optionTitle}>
                    {card.title}
                    {isActive && <span style={{ ...styles.badge, marginLeft: 8 }}>ACTIVE</span>}
                  </span>
                  <span style={styles.optionHint}>{card.hint}</span>
                </button>
              );
            })}

            <button
              type="button"
              className="modal-btn"
              style={{
                ...styles.option,
                ...(uco2Enabled ? styles.optionUco2Active : {}),
                ...(uco2Disabled ? styles.optionDisabled : {}),
              }}
              onClick={onOpenUco2Panel}
              disabled={uco2Disabled}
              title={onlineFixActive ? 'Disable Online Aether first (the two masking pipelines are exclusive)' : 'Open the UCO2 setup panel'}
            >
              <span style={styles.optionTitle}>
                UCO2
                {uco2Enabled && <span style={{ ...styles.badgeUco2, marginLeft: 8 }}>ACTIVE</span>}
              </span>
              <span style={styles.optionHint}>
                Unlock Custom Online 2 for cracked games. Compatible with None and Show Online.
              </span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
