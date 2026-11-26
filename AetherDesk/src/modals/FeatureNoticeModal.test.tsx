import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AddDlcNoticeModal, BackupNoticeModal } from './FeatureNoticeModal';

describe('BackupNoticeModal', () => {
  it('explains the upcoming backup view and where automatic backups live on disk', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<BackupNoticeModal onClose={onClose} />);

    expect(screen.getByText('Backup')).toBeInTheDocument();
    expect(screen.getByText(/not available yet/i)).toBeInTheDocument();
    expect(screen.getByText(/local and cloud backups/i)).toBeInTheDocument();
    expect(screen.getByText(/\.lua\/\.manifest.*achievements.*playtime/i)).toBeInTheDocument();
    expect(screen.getByText('AetherDesk/AetherData/backup/<appid>')).toBeInTheDocument();

    await user.click(screen.getByText('Close'));
    expect(onClose).toHaveBeenCalledOnce();
  });
});

describe('AddDlcNoticeModal', () => {
  it('explains the upcoming Add DLC flow and current Store Lua workaround', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<AddDlcNoticeModal onClose={onClose} />);

    expect(screen.getByText('Add DLCs')).toBeInTheDocument();
    expect(screen.getByText(/upcoming updates/i)).toBeInTheDocument();
    expect(screen.getByText(/not the DLC's \.lua/i)).toBeInTheDocument();
    expect(screen.getByText(/CreamAPI or SmokeAPI/i)).toBeInTheDocument();
    expect(screen.getByText(/depends on the provider rather than Aether/i)).toBeInTheDocument();
    expect(screen.getByText(/Non-owned Lua games already include DLCs/i)).toBeInTheDocument();
    expect(screen.getByText(/in-game or in the game's Steam Properties/i)).toBeInTheDocument();

    await user.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledOnce();
  });
});
