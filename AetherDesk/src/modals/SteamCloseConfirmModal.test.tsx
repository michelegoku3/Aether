import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { SteamCloseConfirmModal } from './SteamCloseConfirmModal';

const renderModal = () => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(<SteamCloseConfirmModal onConfirm={onConfirm} onCancel={onCancel} />);
  return { onConfirm, onCancel };
};

describe('SteamCloseConfirmModal', () => {
  it('says the operation closes Steam and to save an open game first', () => {
    renderModal();
    expect(screen.getByText('This operation will close Steam. Are you sure you want to do it?')).toBeInTheDocument();
    expect(screen.getByText('If you have a game open, save the game and close it.')).toBeInTheDocument();
  });

  it('runs the operation on YES and only dismisses on NO', () => {
    const { onConfirm, onCancel } = renderModal();

    fireEvent.click(screen.getByRole('button', { name: 'No' }));
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onConfirm).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Yes' }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it('lets NO mean something else when the flow provides a rejection handler', () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    const onReject = vi.fn();
    render(<SteamCloseConfirmModal onConfirm={onConfirm} onCancel={onCancel} onReject={onReject} />);

    fireEvent.click(screen.getByRole('button', { name: 'No' }));
    expect(onReject).toHaveBeenCalledOnce();
    expect(onCancel).not.toHaveBeenCalled();
  });

  it('lets X dismiss and abort', () => {
    const { onConfirm, onCancel } = renderModal();
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
