/**
 * Emitted by `commands/aether_desk.rs` while a portable AetherDesk update is
 * downloaded and staged, so the AETHER panel can show what the updater is
 * doing instead of one static line.
 *
 * Payload uses camelCase (`#[serde(rename_all = "camelCase")]` on the Rust
 * side), the IPC convention of `docs/shared_contracts.md` §7.
 */
export const DESK_UPDATE_PROGRESS_EVENT = 'desk-update://progress';

/**
 * Phases in the order they occur. `looking-up` covers both the release lookup
 * and the "is it newer" gate, which are network calls too.
 */
export type DeskUpdatePhase = 'looking-up' | 'downloading' | 'extracting' | 'staging' | 'restarting';

export interface DeskUpdateProgress {
  phase: DeskUpdatePhase;
  /** Bytes written so far. Always 0 outside the download. */
  downloadedBytes: number;
  /** `Content-Length`, when the server sent one. */
  totalBytes: number | null;
}
