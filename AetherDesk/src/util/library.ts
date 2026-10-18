/**
 * Live-state resolution for actions that start from a selected game.
 *
 * Cards and the Home result list hand over the game object they rendered, and
 * the Library keeps scanning behind them: Steam installing, updating or
 * uninstalling a game invalidates the shared list on its own. Acting on that
 * stale snapshot is how a game Steam had just finished installing still
 * answered "the game must be installed first" until the user pressed Refresh.
 *
 * The live entry wins whenever the game is still in the library; the snapshot
 * remains the fallback for a game that just left it (the action then fails with
 * the backend's own message instead of silently doing nothing).
 */
export const resolveLiveGame = <T extends { id: number }>(
  games: readonly T[] | undefined,
  snapshot: T | null
): T | null => {
  if (!snapshot) return null;
  return games?.find((entry) => entry.id === snapshot.id) ?? snapshot;
};
