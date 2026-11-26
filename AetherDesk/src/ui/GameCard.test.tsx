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

  it('marks a library game as In Library ahead of Available and behind Installed', () => {
    const { rerender } = render(<GameCard game={game({ in_library: true, has_manifest: true })} />);

    expect(screen.getByText('In Library')).toBeInTheDocument();
    expect(screen.queryByText('Available')).toBeNull();

    rerender(<GameCard game={game({ in_library: true, installed: true, has_manifest: true })} />);
    expect(screen.getByText('Installed')).toBeInTheDocument();
    expect(screen.queryByText('In Library')).toBeNull();
  });
});

describe('store card tag', () => {
  const tag = () => document.querySelector('.badge-store-tag');

  it('joins UCO2, crack status and DRM into one pipe-separated tag', () => {
    render(<GameCard game={game({ uco2_supported: true, crack_status: 'cracked', has_denuvo: true })} />);

    expect(tag()?.textContent).toBe('UCO2 | Cracked | Denuvo');
  });

  it('keeps Denuvo when a later source reports DRM-Free', () => {
    render(<GameCard game={game({ has_denuvo: true, drm_protection: 'None' })} />);

    expect(tag()?.textContent).toBe('Denuvo');
  });

  it('normalizes misspelled Denuvo', () => {
    render(<GameCard game={game({ drm_protection: 'Enuvo' })} />);

    expect(tag()?.textContent).toBe('Denuvo');
  });

  it('labels hypervisor cracks distinctly', () => {
    render(<GameCard game={game({ crack_status: 'hypervisor' })} />);

    expect(tag()?.textContent).toBe('HV Crack');
  });

  it('shortens noisy DRM labels to DRM', () => {
    render(<GameCard game={game({ drm_protection: 'Offline crack team ==' })} />);

    expect(tag()?.textContent).toBe('DRM');
  });

  it('reports DRM-Free for a plain None', () => {
    render(<GameCard game={game({ drm_protection: 'None' })} />);

    expect(tag()?.textContent).toBe('DRM-Free');
  });

  it('labels a plain Steam-DRM game as Steam DRM', () => {
    render(<GameCard game={game({ drm_protection: 'Steam DRM' })} />);

    expect(tag()?.textContent).toBe('Steam DRM');
  });

  it('renders no tag when the game has no store metadata', () => {
    render(<GameCard game={game({})} />);

    expect(tag()).toBeNull();
  });
});

describe('GameCard price', () => {
  it('renders the formatted price with a discount badge', () => {
    render(
      <GameCard
        game={game({
          price: {
            currency: 'IDR',
            finalCents: 15000000,
            formattedFinal: 'Rp 150.000',
            discountPercent: 25,
          },
        })}
      />
    );

    expect(screen.getByText('Rp 150.000')).toBeInTheDocument();
    expect(screen.getByText('-25%')).toBeInTheDocument();
  });

  it('falls back to the currency code instead of a guessed symbol', () => {
    render(<GameCard game={game({ price: { currency: 'IDR', finalCents: 15000000 } })} />);

    expect(screen.getByText('IDR 150000.00')).toBeInTheDocument();
  });

  it('renders no price line when the backend sent none', () => {
    render(<GameCard game={game({})} />);

    expect(document.querySelector('.game-price')).toBeNull();
  });
});
