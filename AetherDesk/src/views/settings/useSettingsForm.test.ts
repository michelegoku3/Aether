import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { AppSettings } from '../../hooks/useSettings';
import { useSettingsForm } from './useSettingsForm';

const base: AppSettings = {
  hubcap_api_key: 'old-key',
  steam_path: 'C:/Steam',
  show_store_dlcs: false,
  backend_owned: 'keep',
};

describe('useSettingsForm', () => {
  it('applies a persisted snapshot and starts clean', () => {
    const { result } = renderHook(() => useSettingsForm());

    act(() => result.current.applySettings(base));

    expect(result.current.form.apiKey).toBe('old-key');
    expect(result.current.form.steamPath).toBe('C:/Steam');
    expect(result.current.dirty).toBe(false);
  });

  it('updates one typed field and reports dirty', () => {
    const { result } = renderHook(() => useSettingsForm());
    act(() => result.current.applySettings(base));

    act(() => result.current.setField('showStoreDlcs', true));

    expect(result.current.form.showStoreDlcs).toBe(true);
    expect(result.current.dirty).toBe(true);
    expect(result.current.getLocalSettings().backend_owned).toBe('keep');
  });

  it('applies overrides without waiting for React state propagation', () => {
    const { result } = renderHook(() => useSettingsForm());
    act(() => result.current.applySettings(base));

    expect(result.current.getLocalSettings({ steam_path: 'D:/Steam' }).steam_path)
      .toBe('D:/Steam');
  });

  it('updates the baseline after an immediate persistence without resetting the form', () => {
    const { result } = renderHook(() => useSettingsForm());
    act(() => result.current.applySettings(base));
    act(() => result.current.setField('apiKey', 'unsaved'));

    act(() => result.current.acceptPersistedSettings({ ...base, steam_path: 'D:/Steam' }));

    expect(result.current.form.apiKey).toBe('unsaved');
    expect(result.current.dirty).toBe(true);
  });
});
