/**
 * Shared error-message helpers for Tauri IPC failure paths.
 *
 * Every `invoke()` rejection arrives as an untyped value (usually the
 * backend's `String` error, sometimes an `Error`). Catching it as `unknown`
 * and routing through `toErrorMessage` keeps the message identical to the
 * old `String(err)` behaviour while letting TypeScript narrow the type.
 */

/** Best-effort message for any thrown value. Never throws itself. */
export const toErrorMessage = (err: unknown): string => {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  try {
    return String(err);
  } catch {
    return 'Unknown error';
  }
};

/** True when the user cancelled a native dialog (not a real failure). */
export const isUserCancellation = (err: unknown): boolean => toErrorMessage(err).includes('No file selected');

/**
 * Machine-readable error code from a backend `CODE: message` string.
 * Mirrors `codes::split_code` in `src-tauri/src/core/error.rs`: only a
 * `SCREAMING_SNAKE_CASE` prefix of 4+ chars before the first colon counts,
 * so ordinary text (`C:\...`, times, short prefixes) never misfires.
 */
export const getErrorCode = (err: unknown): string | null => {
  const message = toErrorMessage(err);
  const colon = message.indexOf(':');
  if (colon < 0) return null;
  const code = message.slice(0, colon);
  if (!/^[A-Z0-9_]{4,}$/.test(code)) return null;
  return code;
};

/** Known control-flow codes (must stay in sync with `core::error::codes`). */
const ErrorCodes = {
  SETTINGS_CONFLICT: 'SETTINGS_CONFLICT',
  GAME_STATE_CONFLICT: 'GAME_STATE_CONFLICT',
  HUBCAP_KEY_REQUIRED_FOR_UPDATES: 'HUBCAP_KEY_REQUIRED_FOR_UPDATES',
  CONFIG_CONFLICT: 'CONFIG_CONFLICT',
  GAME_BUSY: 'GAME_BUSY',
} as const;

/** True when the backend reports a concurrent settings change. */
export const isSettingsConflict = (err: unknown): boolean => getErrorCode(err) === ErrorCodes.SETTINGS_CONFLICT;

/** True when an update needs a valid Hubcap key first. */
export const needsHubcapKey = (err: unknown): boolean =>
  getErrorCode(err) === ErrorCodes.HUBCAP_KEY_REQUIRED_FOR_UPDATES;
