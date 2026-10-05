// I-F: modale di login LuaTools estratta da SettingsView.tsx. Il flusso
// (scelta OAuth/codice, input codice, errori) vive qui; lo stato e le azioni
// restano nella view perché interagiscono con showStatus e i comandi Tauri.

export interface LuaToolsLoginModalProps {
  open: boolean;
  /** True mentre una sign-in è in corso (OAuth o codice). */
  busy: boolean;
  mode: 'choice' | 'code';
  code: string;
  error: string;
  onClose: () => void;
  onSetMode: (mode: 'choice' | 'code') => void;
  onCodeChange: (code: string) => void;
  onOAuthSignIn: () => void;
  onCodeSignIn: () => void;
}

export function LuaToolsLoginModal({
  open,
  busy,
  mode,
  code,
  error,
  onClose,
  onSetMode,
  onCodeChange,
  onOAuthSignIn,
  onCodeSignIn,
}: LuaToolsLoginModalProps) {
  if (!open) return null;
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-container" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">Login to <strong>LuaTools</strong></span>
          <button
            type="button"
            className="modal-close-btn"
            disabled={busy}
            onClick={onClose}
          >
            &times;
          </button>
        </div>
        <div className="modal-separator"></div>

        <div className="modal-body">
          {mode === 'choice' ? (
            <>
              <p className="settings-desc" style={{ margin: '0 0 8px' }}>
                Choose how to authenticate. Both methods create the same LuaTools session for downloads.
              </p>
              <div style={{ display: 'grid', gap: '10px' }}>
                <button
                  type="button"
                  className="big-action-btn luatools-login-choice"
                  onClick={onOAuthSignIn}
                >
                  <div>
                    <strong>Discord OAuth</strong>
                    <p className="settings-desc" style={{ margin: '5px 0 0' }}>
                      Easiest option. Opens Discord in your browser, but Discord/Supabase may share the email linked to your account.
                    </p>
                  </div>
                </button>
                <button
                  type="button"
                  className="big-action-btn luatools-login-choice"
                  onClick={() => onSetMode('code')}
                >
                  <div>
                    <strong>/login code</strong>
                    <p className="settings-desc" style={{ margin: '5px 0 0' }}>
                      More private: generate a one-time code with @Luie. LuaTools states that only your Discord username is collected, not your Discord email.
                    </p>
                  </div>
                </button>
              </div>
            </>
          ) : (
            <>
              <p className="settings-desc">
                Send <strong>/login</strong> to <strong>@Luie</strong> in Discord, then enter the 6-character code below. Codes are single-use and expire after about 5 minutes.
              </p>
              <input
                className="settings-input"
                type="text"
                inputMode="text"
                autoComplete="one-time-code"
                maxLength={6}
                placeholder="ABC123"
                value={code}
                disabled={busy}
                onChange={(event) => onCodeChange(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    event.preventDefault();
                    onCodeSignIn();
                  }
                }}
                style={{ marginTop: '14px', textTransform: 'uppercase', letterSpacing: '4px', textAlign: 'center' }}
              />
              {error && (
                <div className="settings-alert error" style={{ marginTop: '10px' }}>
                  {error}
                </div>
              )}
              <div className="version-actions" style={{ marginTop: '14px' }}>
                <button
                  type="button"
                  className="panel-btn"
                  disabled={busy || code.length !== 6}
                  onClick={onCodeSignIn}
                >
                  {busy ? 'Connecting...' : 'Connect'}
                </button>
                <button
                  type="button"
                  className="panel-btn"
                  disabled={busy}
                  onClick={() => onSetMode('choice')}
                >
                  Back
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
