import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Sidebar } from './Sidebar';

const renderSidebar = (overrides: Partial<React.ComponentProps<typeof Sidebar>> = {}) =>
  render(
    <Sidebar
      activeTab="home"
      onTabChange={vi.fn()}
      onBackupClick={vi.fn()}
      onSteamAction={vi.fn()}
      steamRunning={false}
      dllUpdateAvailable={false}
      updateIsTest={false}
      {...overrides}
    />
  );

describe('Sidebar', () => {
  it('invokes onBackupClick without switching tabs when Backup is clicked', async () => {
    const user = userEvent.setup();
    const onTabChange = vi.fn();
    const onBackupClick = vi.fn();
    renderSidebar({ onTabChange, onBackupClick });

    const backup = screen.getByRole('button', { name: 'Backup' });
    expect(backup).toBeEnabled();

    await user.click(backup);

    expect(onBackupClick).toHaveBeenCalledOnce();
    expect(onTabChange).not.toHaveBeenCalled();
  });
});
