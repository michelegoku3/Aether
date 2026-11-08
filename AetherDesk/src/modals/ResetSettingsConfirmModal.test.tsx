import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ResetSettingsConfirmModal } from './ResetSettingsConfirmModal';

describe('ResetSettingsConfirmModal', () => {
  it('confirms or cancels the destructive reset', () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(<ResetSettingsConfirmModal busy={false} onConfirm={onConfirm} onCancel={onCancel} />);

    expect(screen.getByText('Reset settings?')).toBeInTheDocument();
    expect(screen.getByText(/cannot be undone/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Yes' }));
    expect(onConfirm).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole('button', { name: 'No' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('locks every action while the reset runs', () => {
    render(<ResetSettingsConfirmModal busy onConfirm={vi.fn()} onCancel={vi.fn()} />);

    expect(screen.getByRole('button', { name: 'Working…' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'No' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Close' })).toBeDisabled();
  });
});
