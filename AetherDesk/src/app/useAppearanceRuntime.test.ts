import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { invokeMock, mockInvokeCommands } from '../test/tauriMocks';
import { useAppearanceRuntime } from './useAppearanceRuntime';

const settings = {
  hubcap_api_key: '',
  steam_path: 'C:/Steam',
  use_alternative_game_cards: true,
  custom_css_enabled: true,
  personal_wallpaper_enabled: true,
  personal_wallpaper_opacity: 45,
  alternative_cards_opacity: 80,
  alternative_cards_fade: 30,
};

describe('useAppearanceRuntime', () => {
  it('applies a shared settings snapshot and publishes readiness/revision', async () => {
    mockInvokeCommands({
      get_custom_css: ':root { --accent: red; }',
      get_personal_wallpaper_data_uri: 'data:image/png;base64,abc',
    });
    const onSettingsApplied = vi.fn();
    const { result } = renderHook(() => useAppearanceRuntime({ onSettingsApplied }));

    await act(() => result.current.refresh(settings));

    expect(result.current.settingsReady).toBe(true);
    expect(result.current.settingsRevision).toBe(1);
    expect(result.current.useAlternativeGameCards).toBe(true);
    expect(result.current.alternativeCardsOpacity).toBe(80);
    expect(result.current.alternativeCardsFade).toBe(30);
    expect(onSettingsApplied).toHaveBeenCalledWith(settings);
    await waitFor(() => expect(document.getElementById('aether-custom-css')).toBeInTheDocument());
    expect(document.getElementById('aether-personal-wallpaper')).toHaveStyle({ opacity: '0.45' });
  });

  it('supports live previews without changing persisted settings revision', () => {
    mockInvokeCommands({ get_personal_wallpaper_data_uri: '' });
    const { result } = renderHook(() => useAppearanceRuntime({ onSettingsApplied: vi.fn() }));

    act(() => result.current.previewPersonalWallpaper(true, 150));
    act(() => result.current.previewAlternativeCards(-5, 120));

    expect(result.current.alternativeCardsOpacity).toBe(0);
    expect(result.current.alternativeCardsFade).toBe(100);
    expect(result.current.settingsRevision).toBe(0);
  });

  it('falls back to defaults and still becomes ready when settings loading fails', async () => {
    invokeMock.mockRejectedValue(new Error('settings unavailable'));
    const { result } = renderHook(() => useAppearanceRuntime({ onSettingsApplied: vi.fn() }));

    await act(() => result.current.refresh());

    expect(result.current.settingsReady).toBe(true);
    expect(result.current.useAlternativeGameCards).toBe(false);
    expect(result.current.alternativeCardsOpacity).toBe(100);
    expect(result.current.alternativeCardsFade).toBe(50);
  });

  it('reloads CSS when the live toggle changes', async () => {
    mockInvokeCommands({ get_custom_css: '.test { color: red; }' });
    const { result } = renderHook(() => useAppearanceRuntime({ onSettingsApplied: vi.fn() }));

    act(() => result.current.changeCustomCss(true));

    await waitFor(() => expect(document.getElementById('aether-custom-css')).toHaveTextContent('.test'));
  });
});
