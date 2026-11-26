import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ChangeVersionModal from './ChangeVersionModal';
import type { LuaManifestRow } from './SpecificVersionModal';
import { invokeMock } from '../test/tauriMocks';

const { useLibraryGamesMock, queryGameStateMock, invalidateGameStateMock } = vi.hoisted(() => ({
  useLibraryGamesMock: vi.fn(),
  queryGameStateMock: vi.fn(),
  invalidateGameStateMock: vi.fn(),
}));

vi.mock('../hooks/useLibraryGames', () => ({ useLibraryGames: useLibraryGamesMock }));

const APP_ID = 480;

const build = { buildId: 24701871, date: '2026-01-15', title: 'Build 24701871' };

interface RenderOptions {
  onClose?: () => void;
  initialTab?: 'manual' | 'auto';
  initialRows?: LuaManifestRow[];
}

const renderModal = ({ onClose = vi.fn(), initialTab = 'auto', initialRows = [] }: RenderOptions = {}) => {
  useLibraryGamesMock.mockReturnValue({
    loadInstalledGames: vi.fn(),
    queryGameState: queryGameStateMock,
    invalidateGameState: invalidateGameStateMock,
  });
  queryGameStateMock.mockImplementation(async (_appId: number, command: string) => {
    if (command === 'get_game_builds') return [build];
    if (command === 'get_saved_builds') return [];
    if (command === 'get_installed_lua_manifest_rows') return initialRows;
    return null;
  });
  render(
    <ChangeVersionModal
      game={{ name: 'Half-Life', appId: String(APP_ID) }}
      initialRows={initialRows}
      initialTab={initialTab}
      onClose={onClose}
    />
  );
};

describe('ChangeVersionModal bookmarks', () => {
  beforeEach(() => {
    invalidateGameStateMock.mockReset();
  });

  it('reports a refused bookmark and keeps the build list', async () => {
    invokeMock.mockRejectedValueOnce(new Error('saved_builds.json is locked'));
    renderModal();
    const user = userEvent.setup();

    await user.click(await screen.findByTitle('Save this build'));

    await screen.findByText('saved_builds.json is locked');
    // The star is still unsaved and still there: the user can try again.
    expect(screen.getByTitle('Save this build')).toBeInTheDocument();
    expect(screen.queryByTitle('Remove from saved builds')).not.toBeInTheDocument();
  });

  it('marks the build as saved when the backend accepts it', async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    renderModal();
    const user = userEvent.setup();

    await user.click(await screen.findByTitle('Save this build'));

    expect(await screen.findByTitle('Remove from saved builds')).toBeInTheDocument();
  });

  it('drops the refusal message when the builds are reloaded', async () => {
    invokeMock.mockRejectedValueOnce(new Error('saved_builds.json is locked'));
    renderModal();
    const user = userEvent.setup();

    await user.click(await screen.findByTitle('Save this build'));
    await screen.findByText('saved_builds.json is locked');

    await user.click(screen.getByTitle('Refresh builds'));

    await waitFor(() => expect(screen.queryByText('saved_builds.json is locked')).not.toBeInTheDocument());
  });
});

describe('ChangeVersionModal tabs', () => {
  it('keeps the Auto tab disabled and marks it as under maintenance', async () => {
    renderModal();
    // Wait for the build list so the tab's queries settle inside the test.
    await screen.findByText('Build #24701871');

    expect(screen.getByRole('tab', { name: 'Auto' })).toBeDisabled();
    // The badge is decorative: the disabled tab and its tooltip carry the
    // state, so it stays out of the accessibility tree.
    expect(screen.getByText('MAINTENANCE')).toHaveAttribute('aria-hidden', 'true');
  });
});

describe('ChangeVersionModal dismissal', () => {
  const row = { rowId: 1, appId: APP_ID, manifestId: '111111', enabled: true };

  it('stays open when the overlay is clicked', async () => {
    const onClose = vi.fn();
    renderModal({ onClose });

    await screen.findByText('Build #24701871');
    fireEvent.click(document.querySelector('.modal-overlay')!);

    expect(onClose).not.toHaveBeenCalled();
  });

  it('ignores Escape while the manual edits are unsaved', async () => {
    const onClose = vi.fn();
    renderModal({ onClose, initialTab: 'manual', initialRows: [row] });

    await waitFor(() => expect(document.querySelector('.version-manifest-input')).not.toBeNull());
    fireEvent.change(document.querySelector('.version-manifest-input')!, { target: { value: '222222' } });
    fireEvent.keyDown(window, { key: 'Escape' });

    expect(onClose).not.toHaveBeenCalled();
  });

  it('closes on Escape when nothing has changed', async () => {
    const onClose = vi.fn();
    renderModal({ onClose, initialTab: 'manual', initialRows: [row] });

    await waitFor(() => expect(document.querySelector('.version-manifest-input')).not.toBeNull());
    fireEvent.keyDown(window, { key: 'Escape' });

    expect(onClose).toHaveBeenCalledOnce();
  });
});

describe('ChangeVersionModal non-host depot guard', () => {
  const hostRow: LuaManifestRow = { rowId: 1, appId: 294104, manifestId: '111111', enabled: true };
  const nonHostRow: LuaManifestRow = {
    rowId: 2,
    appId: 294101,
    manifestId: '222222',
    enabled: true,
    hostSkip: {
      reason: 'wrongArch',
      label: '32-bit',
      detail: 'Depot 294101 targets 32-bit Windows, while a 64-bit depot is present for this game.',
    },
  };

  it('marks non-host rows with the orange class and badge', async () => {
    renderModal({ initialTab: 'manual', initialRows: [hostRow, nonHostRow] });

    expect(await screen.findByText('32-bit')).toBeInTheDocument();
    const tableRows = document.querySelectorAll('.version-table tbody tr');
    expect(tableRows[0]).not.toHaveClass('version-row-nonhost');
    expect(tableRows[1]).toHaveClass('version-row-nonhost');
    expect(document.querySelectorAll('.version-manifest-input')[1]).toHaveAttribute('readonly');
  });

  it('opens the uniform confirm modal on click/type and keeps ChangeVersionModal open on Escape or Cancel', async () => {
    const onClose = vi.fn();
    renderModal({ onClose, initialTab: 'manual', initialRows: [hostRow, nonHostRow] });
    const user = userEvent.setup();

    await screen.findByText('32-bit');
    const nonHostInput = document.querySelectorAll('.version-manifest-input')[1] as HTMLInputElement;
    await user.click(nonHostInput);

    expect(await screen.findByText('Non-applicable depot (32-bit)')).toBeInTheDocument();
    expect(screen.getByText('Depot 294101 is not used on this PC.')).toBeInTheDocument();

    // Escape closes only the stacked confirm modal, not ChangeVersionModal.
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByText('Non-applicable depot (32-bit)')).not.toBeInTheDocument());
    expect(onClose).not.toHaveBeenCalled();

    // Typing also triggers the modal without mutating the input; Cancel keeps it locked.
    fireEvent.keyDown(nonHostInput, { key: '9' });
    expect(await screen.findByText('Non-applicable depot (32-bit)')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(nonHostInput.value).toBe('');
    expect(nonHostInput).toHaveAttribute('readonly');
  });

  it('opens the confirm modal on toggle click and unlocks editing after Edit anyway', async () => {
    renderModal({ initialTab: 'manual', initialRows: [hostRow, nonHostRow] });
    const user = userEvent.setup();

    await screen.findByText('32-bit');
    const switches = document.querySelectorAll('.version-switch');
    const nonHostSwitch = switches[1] as HTMLLabelElement;
    const nonHostToggle = nonHostSwitch.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(nonHostToggle).toBeDisabled();
    await user.click(nonHostSwitch);

    expect(await screen.findByText('Non-applicable depot (32-bit)')).toBeInTheDocument();
    expect(nonHostToggle.checked).toBe(true);

    await user.click(screen.getByRole('button', { name: 'Edit anyway' }));
    await waitFor(() => expect(screen.queryByText('Non-applicable depot (32-bit)')).not.toBeInTheDocument());

    const nonHostInput = document.querySelectorAll('.version-manifest-input')[1] as HTMLInputElement;
    expect(nonHostInput).not.toHaveAttribute('readonly');
    expect(nonHostToggle).not.toBeDisabled();
    await user.type(nonHostInput, '333333');
    expect(nonHostInput.value).toBe('333333');
    await user.click(nonHostSwitch);
    expect(nonHostToggle.checked).toBe(false);
  });
});
