import type { LibraryActionGame } from './LibraryGameActionsModal';
import { useModalDismiss, useOverlayDismiss } from '../hooks/useModalDismiss';
import { resolveOptionState, type AppPresenceMode, type OnlineOptionKey } from './onlineChoiceState';

export type { AppPresenceMode, OnlineOptionKey };

interface OnlineOption {
  key: OnlineOptionKey;
  title: string;
  hint: string;
  accent: 'primary' | 'uco2';
}

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
    key: 'aetheronline',
    title: 'Online Aether',
    hint: 'Full Spacewar mask + friend presence for AetherOnline sessions',
    accent: 'primary',
  },
  {
    key: 'uco2',
    title: 'UCO2',
    hint: 'Unlock Custom Online 2 for cracked games.',
    accent: 'uco2',
  },
] as const;

interface OnlineChoiceModalProps {
  game: LibraryActionGame;
  mode: AppPresenceMode;
  uco2Enabled: boolean;
  ofmePresent: boolean;
  uco2FilesPresent: boolean;
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
  banner: { marginBottom: '12px', padding: '10px 12px', background: '#2a2418', color: '#e0b06a', fontSize: '12px', borderRadius: '8px' },
};

export const OnlineChoiceModal = ({
  game,
  mode,
  uco2Enabled,
  ofmePresent,
  uco2FilesPresent,
  busy,
  onSelectMode,
  onOpenUco2Panel,
  onClose,
}: OnlineChoiceModalProps) => {
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
          {ofmePresent && (
            <div style={styles.banner}>
              OFME crack files detected, keep None to use that crack. Other options are now locked.
            </div>
          )}
          <div style={styles.grid}>
            {OPTIONS.map((option) => {
              const { isActive, isBlocked, tooltip } = resolveOptionState(option.key, {
                mode,
                uco2Enabled,
                ofmePresent,
                uco2FilesPresent,
                busy,
              });
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
