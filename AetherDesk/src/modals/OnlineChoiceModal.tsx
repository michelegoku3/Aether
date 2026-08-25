import type { LibraryActionGame } from './LibraryGameActionsModal';
import { useModalDismiss, useOverlayDismiss } from '../hooks/useModalDismiss';

// Modalità presenza/online per app, allineata 1:1 con gli array [presence] di
// aethercore.toml (docs/05 §12): none = exclude_apps (hard opt-out),
// showonline = showonline_apps, onlinefix = onlinefix_apps.
export type AppPresenceMode = 'none' | 'showonline' | 'onlinefix';

// Le tre modalità Aether sono mutuamente esclusive (radio); UCO2 è un toggle
// ortogonale con la sua pipeline di masking (vedi OnlinePanel). 'uco2' vive in
// questa union perché condivide riga, colonne e allineamento del popup.
export type OnlineOptionKey = AppPresenceMode | 'uco2';

interface OnlineOption {
  key: OnlineOptionKey;
  title: string;
  hint: string;
  /** Quale accento guida selezione e badge: ciano primario per le modalità
   *  presenza, --color-uco2 per il toggle UCO2 (un accento per significato). */
  accent: 'primary' | 'uco2';
}

// Dati di presentazione delle 4 opzioni: un'unica fonte, nessun JSX
// duplicato. L'ordine dell'array è l'ordine visualizzato.
const OPTIONS: readonly OnlineOption[] = [
  {
    key: 'none',
    title: 'None',
    hint: 'Aether ignores this game: no presence, no online.',
    accent: 'primary',
  },
  {
    key: 'showonline',
    title: 'Show Online',
    hint: 'Friends see what you play (presence only, no online features).',
    accent: 'primary',
  },
  {
    key: 'onlinefix',
    title: 'Online Aether',
    hint: 'Full Spacewar mask + friend presence for OnlineFix sessions',
    accent: 'primary',
  },
  {
    key: 'uco2',
    title: 'UCO2',
    hint: 'Unlock Custom Online 2 for cracked games.',
    accent: 'uco2',
  },
] as const;

/** Stato interattivo derivato di un'opzione (funzione pura, testabile):
 *  UCO2 e Online Aether si bloccano a vicenda perché le due masking pipeline
 *  si escludono (docs/05 §12); `busy` blocca tutto il popup. */
const resolveOptionState = (
  option: OnlineOption,
  ctx: { mode: AppPresenceMode; uco2Enabled: boolean; busy: boolean },
): { isActive: boolean; isBlocked: boolean; tooltip: string } => {
  const isActive = option.key === 'uco2' ? ctx.uco2Enabled : ctx.mode === option.key;
  const pipelineConflict =
    (option.key === 'onlinefix' && ctx.uco2Enabled) ||
    (option.key === 'uco2' && ctx.mode === 'onlinefix');

  if (ctx.busy) {
    return { isActive, isBlocked: true, tooltip: 'Busy…' };
  }
  if (pipelineConflict && !isActive) {
    return {
      isActive,
      isBlocked: true,
      tooltip: option.key === 'uco2'
        ? 'Disable Online Aether first (the two masking pipelines are exclusive)'
        : 'Disable UCO2 first',
    };
  }
  if (isActive) {
    return { isActive, isBlocked: false, tooltip: 'Currently active' };
  }
  return {
    isActive,
    isBlocked: false,
    tooltip: option.key === 'uco2' ? 'Open the UCO2 setup panel' : `Switch to ${option.title}`,
  };
};

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
  header: { display: 'flex' as const, justifyContent: 'space-between' as const, alignItems: 'center' as const, padding: '16px 24px', borderBottom: '1px solid #232329' },
  headerTitle: { margin: 0, fontSize: '16px', fontWeight: 700 },
  close: { background: 'none', border: 'none', color: '#999', fontSize: '20px', cursor: 'pointer', lineHeight: 1 },
  body: { padding: '22px 24px' },
  grid: { display: 'grid' as const, gridTemplateColumns: '1fr', gap: '12px' },
};

/**
 * Popup scelta online (segmented control): le tre modalità Aether sono
 * mutuamente esclusive (radio); UCO2 è un toggle ortogonale, gestito dal suo
 * pannello — apribile sempre tranne quando Online Aether è attivo. Cliccare
 * la modalità già attiva è un no-op: per "spegnere" si seleziona None.
 * Tutta la presentazione (colonne fisse, hover neutro, accento di selezione)
 * vive in style.css sotto `.online-choice-*`.
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
  // ESC + click fuori chiudono SOLO questo popup (vedi useOverlayDismiss per
  // lo stopPropagation che protegge la catena Modify → Online → UCO2).
  useModalDismiss(onClose, busy);
  const handleOverlayClick = useOverlayDismiss(onClose, busy);

  return (
    <div className="modal-overlay" onClick={handleOverlayClick}>
      <div
        className="modal-container"
        style={{ width: 600, maxHeight: '86vh' }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={styles.header}>
          <h3 style={styles.headerTitle}>Online: {game.name} ({game.appId})</h3>
          <button type="button" style={styles.close} onClick={onClose} disabled={busy} aria-label="Close">×</button>
        </div>

        <div style={styles.body}>
          <div style={styles.grid}>
            {OPTIONS.map((option) => {
              const { isActive, isBlocked, tooltip } = resolveOptionState(option, { mode, uco2Enabled, busy });
              // UCO2 apre il suo pannello; le modalità presenza si selezionano
              // (no-op se già attive). Il check dentro l'handler restringe il
              // tipo a AppPresenceMode senza cast.
              const handleOptionClick = () => {
                if (option.key === 'uco2') {
                  onOpenUco2Panel();
                  return;
                }
                if (!isActive) onSelectMode(option.key);
              };
              return (
                <button
                  key={option.key}
                  type="button"
                  className={[
                    'online-choice-option',
                    // --selected è il marker comune (esclude l'hover, vedi
                    // style.css); --selected-uco2 è il refinement d'accento.
                    ...(isActive ? ['online-choice-option--selected'] : []),
                    ...(isActive && option.accent === 'uco2' ? ['online-choice-option--selected-uco2'] : []),
                    ...(isBlocked ? ['online-choice-option--blocked'] : []),
                  ].join(' ')}
                  onClick={handleOptionClick}
                  disabled={isBlocked}
                  title={tooltip}
                >
                  <span className="online-choice-option__title">{option.title}</span>
                  <span className="online-choice-option__badge-col">
                    {isActive && (
                      <span className={option.accent === 'uco2' ? 'online-choice-badge online-choice-badge--uco2' : 'online-choice-badge'}>
                        ACTIVE
                      </span>
                    )}
                  </span>
                  <span className="online-choice-option__hint">{option.hint}</span>
                </button>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
};
