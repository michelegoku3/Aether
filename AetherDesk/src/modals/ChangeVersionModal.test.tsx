import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ChangeVersionModal from './ChangeVersionModal';
import { invokeMock } from '../test/tauriMocks';

const { useLibraryGamesMock, queryGameStateMock, invalidateGameStateMock } = vi.hoisted(() => ({
  useLibraryGamesMock: vi.fn(),
  queryGameStateMock: vi.fn(),
  invalidateGameStateMock: vi.fn(),
}));

vi.mock('../hooks/useLibraryGames', () => ({ useLibraryGames: useLibraryGamesMock }));

const APP_ID = 480;

const build = { buildId: 24701871, date: '2026-01-15', title: 'Build 24701871' };

const renderModal = () => {
  useLibraryGamesMock.mockReturnValue({
    loadInstalledGames: vi.fn(),
    queryGameState: queryGameStateMock,
    invalidateGameState: invalidateGameStateMock,
  });
  queryGameStateMock.mockImplementation(async (_appId: number, command: string) => {
    if (command === 'get_game_builds') return [build];
    if (command === 'get_saved_builds') return [];
    return null;
  });
  render(
    <ChangeVersionModal
      game={{ name: 'Half-Life', appId: String(APP_ID) }}
      initialRows={[]}
      initialTab="auto"
      onClose={vi.fn()}
    />
  );
};

describe('ChangeVersionModal bookmarks', () => {
  beforeEach(() => {
    invalidateGameStateMock.mockReset();
  });

  it('reports a refused bookmark and keeps the build list', async () => {
    invokeMock.mockRejectedValueOnce(new Error('saved_builds.json is locked'));
    renderModal();
    const user = userEvent.setup();

    await user.click(await screen.findByTitle('Save this build'));

    await screen.findByText('saved_builds.json is locked');
    // The star is still unsaved and still there: the user can try again.
    expect(screen.getByTitle('Save this build')).toBeInTheDocument();
    expect(screen.queryByTitle('Remove from saved builds')).not.toBeInTheDocument();
  });

  it('marks the build as saved when the backend accepts it', async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    renderModal();
    const user = userEvent.setup();

    await user.click(await screen.findByTitle('Save this build'));

    expect(await screen.findByTitle('Remove from saved builds')).toBeInTheDocument();
  });

  it('drops the refusal message when the builds are reloaded', async () => {
    invokeMock.mockRejectedValueOnce(new Error('saved_builds.json is locked'));
    renderModal();
    const user = userEvent.setup();

    await user.click(await screen.findByTitle('Save this build'));
    await screen.findByText('saved_builds.json is locked');

    await user.click(screen.getByTitle('Refresh builds'));

    await waitFor(() => expect(screen.queryByText('saved_builds.json is locked')).not.toBeInTheDocument());
  });
});

describe('ChangeVersionModal tabs', () => {
  it('keeps the Auto tab disabled and marks it as under maintenance', async () => {
    renderModal();
    // Wait for the build list so the tab's queries settle inside the test.
    await screen.findByText('Build #24701871');

    expect(screen.getByRole('tab', { name: 'Auto' })).toBeDisabled();
    // The badge is decorative: the disabled tab and its tooltip carry the
    // state, so it stays out of the accessibility tree.
    expect(screen.getByText('MAINTENANCE')).toHaveAttribute('aria-hidden', 'true');
  });
});
