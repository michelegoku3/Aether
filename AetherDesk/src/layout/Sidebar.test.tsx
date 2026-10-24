import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Sidebar } from './Sidebar';

const renderSidebar = () =>
  render(
    <Sidebar
      activeTab="home"
      onTabChange={vi.fn()}
      onSteamAction={vi.fn()}
      steamRunning={false}
      dllUpdateAvailable={false}
      updateIsTest={false}
    />
  );

describe('Sidebar', () => {
  it('keeps the disabled Backup entry non-interactive but explains itself', () => {
    renderSidebar();

    const backup = screen.getByRole('button', { name: 'Backup' });
    expect(backup).toBeDisabled();
    // WebView2 suppresses tooltips on disabled controls: the hint lives on the
    // wrapper, not on the button.
    expect(backup).not.toHaveAttribute('title');
    expect(backup.parentElement).toHaveAttribute('title', 'Backup is not available yet');
  });
});
