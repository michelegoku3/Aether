import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { UninstallDeskConfirmModal } from './UninstallDeskModal';

const renderModal = (isProcessing = false) => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(<UninstallDeskConfirmModal isProcessing={isProcessing} onConfirm={onConfirm} onCancel={onCancel} />);
  return { onConfirm, onCancel };
};

describe('UninstallDeskConfirmModal', () => {
  it('describes owned-file removal and leaves retained data in place', () => {
    renderModal();
    expect(screen.getByText(/Only verified AetherDesk application files/)).toHaveTextContent(
      'Unrelated or modified files will be kept.'
    );
    expect(screen.getByText(/User data will stay in AetherData at its current location/)).toBeInTheDocument();
    expect(screen.getByRole('checkbox')).not.toBeChecked();
  });

  it('preserves explicit user-data consent and the boolean confirmation contract', async () => {
    const user = userEvent.setup();
    const { onConfirm } = renderModal();
    await user.click(screen.getByRole('checkbox'));
    expect(screen.getByText(/User data .* will be deleted too/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Yes' }));
    expect(onConfirm).toHaveBeenCalledExactlyOnceWith(true);
  });

  it('keeps confirmation and cancellation locked while processing', () => {
    renderModal(true);
    expect(screen.getByRole('checkbox')).toBeDisabled();
    for (const button of screen.getAllByRole('button')) {
      expect(button).toBeDisabled();
    }
  });
});
