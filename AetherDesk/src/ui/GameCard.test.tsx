import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { GameCard, type GameCardModel } from './GameCard';

// The cover resolver probes for image assets; the badge is what this test is
// about, so the slot is stubbed out.
vi.mock('./GameCover', () => ({ GameCover: () => null }));

const game = (state: Partial<GameCardModel>): GameCardModel => ({
  id: 1,
  name: 'Alpha',
  appId: '1',
  ...state,
});

describe('GameCard install badge', () => {
  it('marks a complete game as Installed', () => {
    render(<GameCard game={game({ installed: true })} />);

    expect(screen.getByText('Installed')).toBeInTheDocument();
    expect(screen.queryByText('Installing')).toBeNull();
  });

  it('marks Steam still writing the game as Installing, ahead of Available', () => {
    render(<GameCard game={game({ installing: true, has_manifest: true })} />);

    expect(screen.getByText('Installing')).toBeInTheDocument();
    expect(screen.queryByText('Available')).toBeNull();
    expect(screen.queryByText('Installed')).toBeNull();
  });
});
