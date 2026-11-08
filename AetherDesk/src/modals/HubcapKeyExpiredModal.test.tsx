import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { HubcapKeyExpiredModal } from './HubcapKeyExpiredModal';

describe('HubcapKeyExpiredModal', () => {
  it('explains a rejected key and offers Settings or Later', async () => {
    const onGoToSettings = vi.fn();
    const onClose = vi.fn();
    render(<HubcapKeyExpiredModal reason="unauthorized" onGoToSettings={onGoToSettings} onClose={onClose} />);

    expect(screen.getByText('Hubcap API Key expired')).toBeInTheDocument();
    expect(screen.getByText(/Hubcap rejected the saved API key/i)).toBeInTheDocument();
    expect(screen.getByText(/Adding games, manifest generation, Workshop mods/i)).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(onGoToSettings).toHaveBeenCalledOnce();

    await userEvent.click(screen.getByRole('button', { name: 'Later' }));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('names an inactive account', () => {
    render(<HubcapKeyExpiredModal reason="account_inactive" onGoToSettings={vi.fn()} onClose={vi.fn()} />);

    expect(screen.getByText(/account behind the saved key is no longer allowed/i)).toBeInTheDocument();
  });
});
