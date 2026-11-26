import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { invokeMock } from '../test/tauriMocks';
import { GameInfoModal } from './GameInfoModal';

vi.mock('../ui/GameCover', () => ({ GameCover: () => null }));

describe('GameInfoModal', () => {
  it('renders DRM notice, Steam Deck status, and opens external links', async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'get_game_info') {
        return {
          appId: 730,
          name: 'Counter-Strike 2',
          platforms: { windows: true, linux: true, steamDeckCompatCategory: 2 },
          appDetails: {
            drmNotice: 'Denuvo Anti-Tamper<br>5 machine activation limit',
          },
        };
      }
      return null;
    });

    render(<GameInfoModal appId={730} fallbackName="CS2" onClose={() => {}} />);

    expect(await screen.findByText('Playable')).toBeInTheDocument();
    expect(screen.getByText('Denuvo Anti-Tamper · 5 machine activation limit')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'SteamDB' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('open_game_info_link', { appId: 730, site: 'steamdb' })
    );

    await user.click(screen.getByRole('button', { name: 'PCGamingWiki' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('open_game_info_link', { appId: 730, site: 'pcgw' }));
  });
});
