import { GameCover } from '../GameCover';

export interface GameCardModel {
  id: number;
  name: string;
  appId: string;
  imageUrl?: string;
  has_manifest?: boolean;
  has_denuvo?: boolean;
  installed?: boolean;
}

interface GameCardProps<T extends GameCardModel> {
  game: T;
  actionLabel: string;
  onAction: (game: T) => void;
}

export const GameCard = <T extends GameCardModel>({ game, actionLabel, onAction }: GameCardProps<T>) => {
  return (
    <div key={game.id} className="store-game-card">
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
