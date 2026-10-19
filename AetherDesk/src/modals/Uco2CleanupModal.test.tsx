import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Uco2CleanupModal } from './Uco2CleanupModal';

const CONFIRM_TEXT =
  'No recovery journal found. AetherDesk can remove the known UCOnline2 files, but some legitimate game files may also be removed. You can easily restore them by verifying the game file integrity. Continue?';

describe('Uco2CleanupModal', () => {
  it('asks before the destructive journal-less cleanup and can be cancelled', async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onClose = vi.fn();
    render(<Uco2CleanupModal mode="confirm" text={CONFIRM_TEXT} onConfirm={onConfirm} onClose={onClose} />);

    expect(screen.getByText(CONFIRM_TEXT)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onClose).toHaveBeenCalledOnce();
    expect(onConfirm).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Continue' }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it('locks every action while the cleanup runs', () => {
    render(<Uco2CleanupModal mode="confirm" text={CONFIRM_TEXT} busy onConfirm={vi.fn()} onClose={vi.fn()} />);

    expect(screen.getByRole('button', { name: 'Removing…' })).toBeDisabled();
    for (const button of screen.getAllByRole('button')) {
      expect(button).toBeDisabled();
    }
  });

  it('reports the verify-integrity warning after the cleanup', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(
      <Uco2CleanupModal
        mode="warning"
        text="UCO2 files were removed. Verify integrity of game files."
        onClose={onClose}
      />
    );

    expect(screen.getByText(/Verify integrity of game files/)).toBeInTheDocument();
    await user.click(screen.getByText('Close'));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
