import { memo } from 'react';
import { priceSummary } from '../modals/gameInfoFormat';
import type { GameInfoPrice } from '../modals/gameInfoTypes';
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
  /** True when the game is present in the user's Aether Lua library (Store cards). */
  in_library?: boolean;
  /** Steam price when the backend supplied one (store cards). */
  price?: GameInfoPrice | null;
  /** UCO2 support flag (library games). */
  uco2_supported?: boolean;
  /** Crack status from isitcracked.com (cracked/uncracked/unreleased/...). */
  crack_status?: string;
  /** DRM protection reported by isitcracked.com (e.g. Denuvo, Steam). */
  drm_protection?: string;
  /**
   * Steam has an appmanifest for this game but is still writing its content:
   * the card shows `Installing` instead of `Installed`/`Available`.
   */
  installing?: boolean;
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
const hasActiveLuaIssue = (game: GameCardModel) => (game.luaIssues ?? []).some((issue) => issue.active !== false);

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

const GameCardInner = <T extends GameCardModel>({
  game,
  actionLabel,
  onAction,
  actions,
  onFixPins,
  cardVariant = 'classic',
}: GameCardProps<T>) => {
  const marker = markerClass(game);
  // Pins can only be broken in a Lua that exists: this is false for a game
  // that is mere `Available` (nothing downloaded yet, so no file to repair).
  const pinsToFix = hasActiveLuaIssue(game);
  const fixPinsClass = [
    'badge-fix-pins',
    // Installed does not get a badge of its own next to this one: the green
    // outline carries that information, so the corner keeps a single badge.
    game.installed ? 'badge-fix-pins--installed' : '',
  ]
    .filter(Boolean)
    .join(' ');
  const isBackdrop = cardVariant === 'backdrop';
  const resolvedActions =
    actions ??
    (actionLabel && onAction ? [{ label: actionLabel, onClick: onAction, variant: 'primary' as const }] : []);
  const className = ['store-game-card', marker, isBackdrop ? 'backdrop-card' : ''].filter(Boolean).join(' ');
  const backdropUrl = game.heroImageUrl || game.imageUrl;

  return (
    <div key={game.id} className={className} title={markerTooltip(game)}>
      {isBackdrop && backdropUrl && (
        <div className="game-card-backdrop-bg" style={{ backgroundImage: `url("${backdropUrl}")` }} />
      )}

      {/* One badge per card, in the top-right corner, by urgency: a card with
          pins to fix shows that single badge and hands over the `Installed`
          signal through its green outline — repairing the Lua is the only
          thing left to do on that game. Without pins to fix the corner is
          what it always was: Installed, otherwise Installing while Steam is
          still writing the content, otherwise Available. */}
      {pinsToFix &&
        (onFixPins ? (
          <button type="button" className={fixPinsClass} onClick={() => onFixPins(game)}>
            Fix Pins
          </button>
        ) : (
          <span className={fixPinsClass}>Fix Pins</span>
        ))}

      {!pinsToFix && game.installed && <span className="badge-installed">Installed</span>}

      {!pinsToFix && !game.installed && game.installing && (
        <span className="badge-installing" title="Steam is still downloading this game">
          Installing
        </span>
      )}

      {!pinsToFix && !game.installed && !game.installing && game.in_library && (
        <span className="badge-in-library" title="Already in your Aether Library">
          In Library
        </span>
      )}

      {!pinsToFix && !game.installed && !game.installing && !game.in_library && game.has_manifest && (
        <span
          className={`badge-available ${game.has_denuvo ? 'denuvo' : ''}`}
          title={game.has_denuvo ? 'Available on Hubcap, Denuvo detected' : 'Available on Hubcap'}
        >
          Available
        </span>
      )}

      {!isBackdrop && <GameCover appId={game.appId} name={game.name} canonicalUrl={game.imageUrl} />}

      <div className="game-info-wrapper">
        <div className="game-details">
          <h3 className="game-name" title={game.name}>
            {game.name}
          </h3>
          <span className="game-appid">App ID: {game.appId}</span>
          {game.price && (
            <span className="game-price" title={priceSummary(game.price)}>
              {priceSummary(game.price)}
              {typeof game.price.discountPercent === 'number' && game.price.discountPercent > 0 && (
                <span className="game-price-discount">-{game.price.discountPercent}%</span>
              )}
            </span>
          )}
          {(() => {
            const parts: string[] = [];
            if (game.uco2_supported) parts.push('UCO2');
            if (game.crack_status) {
              const cs = game.crack_status.trim().toLowerCase();
              const label = cs === 'hypervisor' ? 'HV Crack' : cs.charAt(0).toUpperCase() + cs.slice(1);
              parts.push(label);
            }
            // `drm_protection` is optional: undefined means "not checked", which
            // must NOT render as DRM-Free (only an explicit "None" does).
            let drmLabel: string | undefined;
            if (game.has_denuvo) drmLabel = 'Denuvo';
            else if (game.drm_protection) {
              const raw = game.drm_protection.trim();
              const d = raw.toLowerCase();
              if (d === 'none') drmLabel = 'DRM-Free';
              else if (d === 'steam' || d === 'steam drm') drmLabel = 'Steam DRM';
              else if (d.includes('denuvo') || d.includes('enuvo') || d.includes('nuvo')) drmLabel = 'Denuvo';
              else if (d.includes('epic')) drmLabel = 'Epic';
              else if (d === 'ea' || d.includes('origin') || d.includes('ea app')) drmLabel = 'EA';
              else if (/team|offline|ffline|=/.test(raw) || raw.length > 30) drmLabel = 'DRM';
              else drmLabel = raw;
            }
            if (drmLabel) parts.push(drmLabel);
            if (parts.length === 0) return null;
            return (
              <div className="game-label-badges" style={{ display: 'flex', gap: 4, marginTop: 4 }}>
                <span className="badge-store-tag" title={parts.join(' | ')}>
                  {parts.join(' | ')}
                </span>
              </div>
            );
          })()}
        </div>
        <div className="game-card-actions">
          {resolvedActions.map((action) => (
            <button
              key={action.label}
              onClick={() => action.onClick(game)}
              className={['game-download-btn', action.variant === 'secondary' ? 'secondary' : '']
                .filter(Boolean)
                .join(' ')}
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

/**
 * A game card, memoized.
 *
 * `memo` + cast: `memo()` on a generic component collapses `T` to the constraint
 * (`GameCardModel`) and breaks inference at call sites — action callbacks would
 * lose the game's concrete type (`InstalledGame`, `StoreGame`, …). Casting to
 * `typeof GameCardInner` gives TypeScript back the generic signature; at runtime
 * the component stays memoized. The standard idiom for generic React components.
 *
 * Why it matters: the Library grid re-renders on every search keystroke, every
 * toast and every popup open/close. With stable props (`game` from the provider
 * snapshot, `actions` and `onFixPins` memoized in LibraryView) the N cards do
 * not re-render: only the toolbar does.
 */
export const GameCard = memo(GameCardInner) as typeof GameCardInner;
