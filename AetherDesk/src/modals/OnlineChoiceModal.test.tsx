import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { OnlineChoiceModal } from './OnlineChoiceModal';
import type { LibraryActionGame } from './LibraryGameActionsModal';

const game: LibraryActionGame = {
  id: 1,
  name: 'Game',
  appId: '1',
  installDir: '',
  libraryPath: '',
  gamePath: '',
  installed: true,
  installing: false,
};

const renderModal = (ofmePresent: boolean, onClean = vi.fn()) => {
  render(
    <OnlineChoiceModal
      game={game}
      mode="none"
      uco2Enabled={false}
      ofmePresent={ofmePresent}
      uco2FilesPresent={false}
      busy={false}
      onSelectMode={vi.fn()}
      onOpenUco2Panel={vi.fn()}
      onClean={onClean}
      onClose={vi.fn()}
    />
  );
  return { onClean };
};

describe('OnlineChoiceModal OFME banner', () => {
  it('shows a Clean button that triggers the cleanup', async () => {
    const user = userEvent.setup();
    const { onClean } = renderModal(true);

    expect(screen.getByText(/OFME crack files detected/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Clean' }));
    expect(onClean).toHaveBeenCalledOnce();
  });

  it('hides the banner and Clean when no OFME files are present', () => {
    renderModal(false);
    expect(screen.queryByRole('button', { name: 'Clean' })).not.toBeInTheDocument();
    expect(screen.queryByText(/OFME crack files detected/)).not.toBeInTheDocument();
  });
});
