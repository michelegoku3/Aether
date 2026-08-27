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
    hint: 'Friends see what you play: presence only, no online.',
    accent: 'primary',
  },
  {
    key: 'aetheronline',
    title: 'Online Aether',
    hint: 'Full 480 Spacewar mask: friend presence, yes online.',
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
        className="modal-container oc-container"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="oc-header">
          <h3 className="oc-header-title">Online: {game.name} ({game.appId})</h3>
          <button type="button" className="oc-close" onClick={onClose} disabled={busy} aria-label="Close">×</button>
        </div>

        <div className="oc-body">
          {ofmePresent && (
            <div className="oc-banner">
              OFME crack files detected, keep None to use that crack. Other options are now locked.
            </div>
          )}
          <div className="oc-grid">
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
