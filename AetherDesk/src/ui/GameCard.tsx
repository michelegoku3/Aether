import { GameCover } from './GameCover';

export interface GameCardModel {
  id: number;
  name: string;
  appId: string;
  imageUrl?: string;
  heroImageUrl?: string;
  has_manifest?: boolean;
  has_denuvo?: boolean;
  has_nsfw?: boolean;
  has_delisted?: boolean;
  installed?: boolean;
  /**
   * Malformed `setManifestid` lines found in this game's Lua. A single bad
   * line makes AetherDLL reject the whole file, so the card is marked red:
   * the game is installed and looks fine, but every depot override of it is
   * inactive until the Lua is repaired.
   */
  luaIssues?: {
    line: number;
    depotId: number | null;
    rawValue: string;
    problem: string;
    active?: boolean;
  }[];
}

export interface GameCardAction<T extends GameCardModel> {
  label: string;
  onClick: (game: T) => void;
  variant?: 'primary' | 'secondary';
  title?: string;
}

interface GameCardProps<T extends GameCardModel> {
  game: T;
  actionLabel?: string;
  onAction?: (game: T) => void;
  actions?: Array<GameCardAction<T>>;
  cardVariant?: 'classic' | 'backdrop';
  /**
   * Opens the repair flow (version editor) for a game whose Lua has a
   * malformed pin. Optional: without it the badge still marks the problem but
   * is not clickable, so the card stays usable in read-only views.
   */
  onFixPins?: (game: T) => void;
}

/**
 * True when this game's Lua has a malformed pin that the script engine
 * actually executes: AetherDLL rejects the whole file, so every depot
 * override of the game is inactive until the line is repaired.
 */
const hasActiveLuaIssue = (game: GameCardModel) =>
  (game.luaIssues ?? []).some((issue) => issue.active !== false);

/**
 * Card border colour. Only the catalog markers live here now — a broken Lua
 * is signalled by its own red `Fix Pins` badge, so a broken NSFW game keeps
 * showing its content-safety border instead of one marker hiding the other.
 * Priority between the two catalog markers: NSFW over delisted, because a
 * content-safety signal must never be masked by a catalog one.
 */
const markerClass = (game: GameCardModel) => {
  if (game.has_nsfw) return 'nsfw';
  if (game.has_delisted) return 'delisted';
  return '';
};

const markerTooltip = (game: GameCardModel) => {
  const labels = [
    game.has_nsfw ? 'Adult-only (NSFW) content' : null,
    game.has_delisted ? 'Delisted from Steam' : null,
  ].filter(Boolean);
  return labels.length > 0 ? labels.join(' • ') : undefined;
};

export const GameCard = <T extends GameCardModel>({ game, actionLabel, onAction, actions, onFixPins, cardVariant = 'classic' }: GameCardProps<T>) => {
  const marker = markerClass(game);
  // Pins can only be broken in a Lua that exists: this is false for a game
  // that is mere `Available` (nothing downloaded yet, so no file to repair).
  const pinsToFix = hasActiveLuaIssue(game);
  const fixPinsClass = [
    'badge-fix-pins',
    // Installed does not get a badge of its own next to this one: the green
    // outline carries that information, so the corner keeps a single badge.
    game.installed ? 'badge-fix-pins--installed' : '',
  ].filter(Boolean).join(' ');
  const isBackdrop = cardVariant === 'backdrop';
  const resolvedActions = actions ?? (actionLabel && onAction
    ? [{ label: actionLabel, onClick: onAction, variant: 'primary' as const }]
    : []);
  const className = [
    'store-game-card',
    marker,
    isBackdrop ? 'backdrop-card' : '',
  ].filter(Boolean).join(' ');
  const backdropUrl = game.heroImageUrl || game.imageUrl;

  return (
    <div
      key={game.id}
      className={className}
      title={markerTooltip(game)}
    >
      {isBackdrop && backdropUrl && (
        <div className="game-card-backdrop-bg" style={{ backgroundImage: `url("${backdropUrl}")` }} />
      )}

      {/* One badge per card, in the top-right corner, by urgency: a card with
          pins to fix shows that single badge and hands over the `Installed`
          signal through its green outline — repairing the Lua is the only
          thing left to do on that game. Without pins to fix the corner is
          what it always was: Installed, else Available. */}
      {pinsToFix && (onFixPins ? (
        <button
          type="button"
          className={fixPinsClass}
          onClick={() => onFixPins(game)}
        >
          Fix Pins
        </button>
      ) : (
        <span className={fixPinsClass}>Fix Pins</span>
      ))}

      {!pinsToFix && game.installed && (
        <span className="badge-installed">Installed</span>
      )}

      {!pinsToFix && !game.installed && game.has_manifest && (
        <span
          className={`badge-available ${game.has_denuvo ? 'denuvo' : ''}`}
          title={game.has_denuvo ? 'Denuvo DRM detected' : 'Manifest available'}
        >
          Available
        </span>
      )}

      {!isBackdrop && (
        <GameCover appId={game.appId} name={game.name} canonicalUrl={game.imageUrl} />
      )}

      <div className="game-info-wrapper">
        <div className="game-details">
          <h3 className="game-name" title={game.name}>{game.name}</h3>
          <span className="game-appid">App ID: {game.appId}</span>
        </div>
        <div className="game-card-actions">
          {resolvedActions.map((action) => (
            <button
              key={action.label}
              onClick={() => action.onClick(game)}
              className={[
                'game-download-btn',
                action.variant === 'secondary' ? 'secondary' : '',
              ].filter(Boolean).join(' ')}
              title={action.title}
            >
              {action.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
};
