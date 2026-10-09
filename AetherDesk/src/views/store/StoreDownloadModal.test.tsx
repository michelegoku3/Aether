import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { StoreGameResult } from '../../hooks/useStoreSearch';
import { StoreDownloadModal } from './StoreDownloadModal';

const game: StoreGameResult = {
  id: 10,
  appId: '10',
  name: 'Test Game',
  has_manifest: true,
  has_denuvo: false,
};

const renderModal = (isDownloading = false) => {
  const callbacks = {
    onSourceChange: vi.fn(),
    onDownloadLatest: vi.fn(),
    onDownloadSpecific: vi.fn(),
    onClose: vi.fn(),
  };
  const view = render(
    <StoreDownloadModal
      game={game}
      source="hubcap"
      status={{ text: '', type: 'info' }}
      isDownloading={isDownloading}
      {...callbacks}
    />,
  );
  return { ...view, ...callbacks };
};

describe('StoreDownloadModal', () => {
  it('shows game identity and changes provider', async () => {
    const user = userEvent.setup();
    const view = renderModal();

    expect(screen.getByText(/Test Game/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Ryuu' }));

    expect(view.onSourceChange).toHaveBeenCalledWith('ryuu');
    expect(screen.getByRole('button', { name: 'MOED' })).toBeDisabled();
  });

  it('dispatches latest and specific actions', async () => {
    const user = userEvent.setup();
    const view = renderModal();

    await user.click(screen.getByText('Download Latest Version').closest('button')!);
    await user.click(screen.getByText('Download Specific Version').closest('button')!);

    expect(view.onDownloadLatest).toHaveBeenCalledOnce();
    expect(view.onDownloadSpecific).toHaveBeenCalledOnce();
  });

  it('locks close, providers and actions while downloading', () => {
    const view = renderModal(true);
    const buttons = screen.getAllByRole('button');

    expect(buttons.every((button) => button.hasAttribute('disabled'))).toBe(true);
    view.container.querySelector('.modal-overlay')?.dispatchEvent(
      new MouseEvent('click', { bubbles: true }),
    );
    expect(view.onClose).not.toHaveBeenCalled();
  });

  it('closes through the overlay while idle', async () => {
    const user = userEvent.setup();
    const view = renderModal();

    await user.click(view.container.querySelector('.modal-overlay')!);

    expect(view.onClose).toHaveBeenCalledOnce();
  });
});
