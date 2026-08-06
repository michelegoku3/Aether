import { GameCover } from './GameCover';

export interface GameCardModel {
  id: number;
  name: string;
  appId: string;
  imageUrl?: string;
  has_manifest?: boolean;
  has_denuvo?: boolean;
  has_nsfw?: boolean;
  has_delisted?: boolean;
  installed?: boolean;
}

interface GameCardProps<T extends GameCardModel> {
  game: T;
  actionLabel: string;
  onAction: (game: T) => void;
}

/** Marker priority: the NSFW pink border wins over the delisted white one —
    a content-safety signal must never be masked by a catalog one. */
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

export const GameCard = <T extends GameCardModel>({ game, actionLabel, onAction }: GameCardProps<T>) => {
  const marker = markerClass(game);
  return (
    <div
      key={game.id}
      className={marker ? `store-game-card ${marker}` : 'store-game-card'}
      title={markerTooltip(game)}
    >
      {game.has_manifest && (
        <span
          className={`badge-available ${game.has_denuvo ? 'denuvo' : ''}`}
          title={game.has_denuvo ? 'Denuvo DRM detected' : 'Manifest available'}
        >
          Available
        </span>
      )}

      {game.installed && (
        <span className="badge-installed">Installed</span>
      )}

      <GameCover appId={game.appId} name={game.name} canonicalUrl={game.imageUrl} />

      <div className="game-info-wrapper">
        <div className="game-details">
          <h3 className="game-name" title={game.name}>{game.name}</h3>
          <span className="game-appid">App ID: {game.appId}</span>
        </div>
        <button
          onClick={() => onAction(game)}
          className="game-download-btn"
        >
          {actionLabel}
        </button>
      </div>
    </div>
  );
};
