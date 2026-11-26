import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { invokeMock } from '../test/tauriMocks';
import { BackupView } from './BackupView';

const sampleRows = [
  {
    app_id: 730,
    game_name: 'Counter-Strike 2',
    has_lua_original: true,
    history_count: 5,
    manifest_count: 2,
    has_crack: false,
    has_original_files: false,
    has_achievements: true,
    total_size_bytes: 2048,
    backup_path: 'C:/AetherData/backup/730',
  },
  {
    app_id: 570,
    game_name: 'Dota 2',
    has_lua_original: true,
    history_count: 1,
    manifest_count: 1,
    has_crack: false,
    has_original_files: false,
    has_achievements: false,
    total_size_bytes: 1024 * 1024,
    backup_path: 'C:/AetherData/backup/570',
  },
];

describe('BackupView', () => {
  it('displays game names, formatted sizes, filtering, pruning, and two-step delete', async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'list_backup_games') return sampleRows;
      if (command === 'prune_backup_lua_history') return 2;
      if (command === 'delete_game_backup') return 'Deleted backup for App ID 570.';
      return null;
    });

    render(<BackupView />);

    expect(await screen.findByText('Counter-Strike 2')).toBeInTheDocument();
    expect(screen.getByText('Dota 2')).toBeInTheDocument();
    expect(screen.getByText('2.0 KB')).toBeInTheDocument();
    expect(screen.getByText('1.0 MB')).toBeInTheDocument();

    // Only Counter-Strike 2 has > 3 history entries, so only one Prune button is shown.
    const pruneButtons = screen.getAllByRole('button', { name: 'Prune' });
    expect(pruneButtons).toHaveLength(1);
    await user.click(pruneButtons[0]);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('prune_backup_lua_history', { appId: 730, keepLatest: 3 })
    );

    // Filter by App ID.
    const filterInput = screen.getByPlaceholderText(/Filter by game or App ID/);
    await user.type(filterInput, '570');
    expect(screen.queryByText('Counter-Strike 2')).not.toBeInTheDocument();
    expect(screen.getByText('Dota 2')).toBeInTheDocument();

    // Two-step delete confirmation.
    await user.click(screen.getByRole('button', { name: 'Delete' }));
    expect(invokeMock).not.toHaveBeenCalledWith('delete_game_backup', expect.anything());

    await user.click(screen.getByRole('button', { name: 'Confirm' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('delete_game_backup', { appId: 570 }));
  });
});
