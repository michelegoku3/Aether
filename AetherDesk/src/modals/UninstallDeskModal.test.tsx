import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ResetSteamConfirmModal, UninstallDeskConfirmModal } from './UninstallDeskModal';

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

describe('ResetSteamConfirmModal', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  const renderResetModal = (isProcessing = false) => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(<ResetSteamConfirmModal isProcessing={isProcessing} onConfirm={onConfirm} onCancel={onCancel} />);
    return { onConfirm, onCancel };
  };

  it('warns that the reset cannot be undone and lists what will be lost', () => {
    renderResetModal();
    expect(screen.getByText(/cannot be undone/)).toBeInTheDocument();
    expect(screen.getByText(/.lua files/)).toBeInTheDocument();
    expect(screen.getByText(/.manifest files/)).toBeInTheDocument();
  });

  it('keeps YES disabled for the 10-second countdown, then enables it', () => {
    const { onConfirm } = renderResetModal();
    const yesButton = screen.getByRole('button', { name: /Yes, clean Steam/ });

    expect(yesButton).toBeDisabled();
    expect(yesButton).toHaveTextContent('Yes, clean Steam (10)');

    // Step second-by-second: like real browser macrotasks, each tick needs
    // its own flush so the next timeout is scheduled before time advances.
    for (let elapsed = 0; elapsed < 9; elapsed += 1) {
      act(() => {
        vi.advanceTimersByTime(1000);
      });
    }
    expect(yesButton).toBeDisabled();
    expect(yesButton).toHaveTextContent('Yes, clean Steam (1)');

    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(yesButton).toBeEnabled();
    expect(yesButton).toHaveTextContent('Yes, clean Steam');

    fireEvent.click(yesButton);
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it('lets NO abort at any time, even during the countdown', () => {
    const { onConfirm, onCancel } = renderResetModal();
    fireEvent.click(screen.getByRole('button', { name: 'No' }));
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('lets X abort at any time, even during the countdown', () => {
    const { onConfirm, onCancel } = renderResetModal();
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('locks every action while processing', () => {
    renderResetModal(true);
    expect(screen.getByRole('button', { name: 'Working…' })).toBeDisabled();
    for (const button of screen.getAllByRole('button')) {
      expect(button).toBeDisabled();
    }
  });
});
