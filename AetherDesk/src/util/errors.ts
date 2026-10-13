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
export const isUserCancellation = (err: unknown): boolean =>
  toErrorMessage(err).includes('No file selected');
