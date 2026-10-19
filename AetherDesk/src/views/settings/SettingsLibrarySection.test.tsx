import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { SettingsLibrarySection } from './SettingsLibrarySection';

const accounts = [
  { steamId: '76561199523209859', accountName: 'michelegoku3', personaName: 'michelegoku3' },
  { steamId: '76561199876393402', accountName: 'robspont15', personaName: 'SyntaxCode15' },
];

const baseProps = () => ({
  enabled: false,
  setEnabled: vi.fn(),
  appIds: '',
  setAppIds: vi.fn(),
  steamId: '',
  setSteamId: vi.fn(),
  accounts,
  accountsLoaded: true,
  accountsError: false,
});

describe('SettingsLibrarySection', () => {
  it('hides the App ID and account fields while the banner is off', () => {
    const props = baseProps();
    const { container } = render(<SettingsLibrarySection {...props} />);

    expect(container.querySelector('input[type="text"]')).toBeNull();
    expect(container.querySelector('select')).toBeNull();
  });

  it('only flips the form field on toggle: nothing is persisted here', () => {
    const props = baseProps();
    const { container } = render(<SettingsLibrarySection {...props} />);

    fireEvent.click(container.querySelector('input[type="checkbox"]')!);

    expect(props.setEnabled).toHaveBeenCalledWith(true);
  });

  it('shows a single-line App ID input and an account dropdown when enabled', () => {
    const { container } = render(<SettingsLibrarySection {...baseProps()} enabled />);

    const input = container.querySelector('input[type="text"]') as HTMLInputElement;
    expect(input.tagName).toBe('INPUT');
    expect(container.querySelector('textarea')).toBeNull();
    expect(container.querySelectorAll('select option')).toHaveLength(3); // placeholder + 2 accounts
  });

  it('reports invalid tokens inline and keeps valid ones unchecked', () => {
    const { container } = render(<SettingsLibrarySection {...baseProps()} enabled appIds="4656000 4108000" />);

    expect(container.querySelector('.settings-error')?.textContent).toContain('4656000 4108000');
  });

  it('shows no error while every token is a valid App ID', () => {
    const { container } = render(<SettingsLibrarySection {...baseProps()} enabled appIds="4656000, 4108000" />);

    expect(container.querySelector('.settings-error')).toBeNull();
  });

  it('keeps a saved account selectable even when the list is unavailable', () => {
    const { container } = render(
      <SettingsLibrarySection {...baseProps()} enabled accounts={[]} steamId="76561199876393402" />
    );

    expect(container.querySelectorAll('select option')).toHaveLength(2); // placeholder + saved value
    expect(screen.getByRole('option', { name: '76561199876393402' })).toBeInTheDocument();
  });

  it('selects the chosen account', () => {
    const { container } = render(<SettingsLibrarySection {...baseProps()} enabled steamId="76561199876393402" />);

    expect((container.querySelector('select') as HTMLSelectElement).value).toBe('76561199876393402');
  });

  it('distinguishes an unreadable account list from an empty one', () => {
    const failed = render(<SettingsLibrarySection {...baseProps()} enabled accounts={[]} accountsError />);
    expect(failed.container.querySelector('.settings-error')?.textContent).toContain(
      'Could not read the local Steam accounts'
    );

    const empty = render(<SettingsLibrarySection {...baseProps()} enabled accounts={[]} />);
    expect(empty.container.textContent).toContain('No local Steam accounts found');
    expect(empty.container.querySelector('.settings-error')).toBeNull();
  });

  it('does not repeat the removed owner description', () => {
    const { container } = render(<SettingsLibrarySection {...baseProps()} enabled />);

    expect(container.textContent).not.toContain('A real account is required');
  });
});
