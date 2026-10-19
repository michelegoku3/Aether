/**
 * Live-state resolution for actions that start from a selected game.
 *
 * Cards and the Home result list hand over the game object they rendered, but
 * the Library keeps scanning behind them, so that snapshot can be stale (Steam
 * installing, updating or uninstalling a game invalidates the shared list).
 *
 * The live entry wins whenever the game is still in the library; the snapshot is
 * the fallback for a game that just left it, so the action fails with the
 * backend's own message instead of silently doing nothing.
 */
export const resolveLiveGame = <T extends { id: number }>(
  games: readonly T[] | undefined,
  snapshot: T | null
): T | null => {
  if (!snapshot) return null;
  return games?.find((entry) => entry.id === snapshot.id) ?? snapshot;
};
