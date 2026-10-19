import { describe, expect, it } from 'vitest';
import type { AppSettings } from '../../hooks/useSettings';
import { buildSettings, isFormDirty, settingsToForm } from './settingsModel';

const settings = (overrides: Partial<AppSettings> = {}): AppSettings => ({
  hubcap_api_key: 'hubcap-key',
  steam_path: 'C:/Steam',
  ...overrides,
});

describe('settingsModel', () => {
  it('normalizes backend defaults exactly once', () => {
    const form = settingsToForm(settings());

    expect(form).toMatchObject({
      showStoreDlcs: false,
      showStoreNsfw: true,
      showStoreDelisted: true,
      workshopAutoDownloadContent: true,
      showStoreFrontGames: true,
      storeCurrency: 'eur',
      personalWallpaperOpacity: 20,
      alternativeCardsOpacity: 100,
      alternativeCardsFade: 50,
    });
  });

  it('normalizes invalid percentages and unsupported currencies', () => {
    const form = settingsToForm(
      settings({
        personal_wallpaper_opacity: 150,
        alternative_cards_opacity: -20,
        alternative_cards_fade: Number.NaN,
        store_currency: 'unsupported',
      })
    );

    expect(form.personalWallpaperOpacity).toBe(100);
    expect(form.alternativeCardsOpacity).toBe(0);
    expect(form.alternativeCardsFade).toBe(0);
    expect(form.storeCurrency).toBe('eur');
  });

  it('trims custom game names when loading and saving', () => {
    const base = settings({ custom_game_name: '  Aether  ' });
    const form = settingsToForm(base);

    expect(form.customGameName).toBe('Aether');
    expect(buildSettings({ ...form, customGameName: '  Updated  ' }, base).custom_game_name).toBe('Updated');
  });

  it('preserves backend-owned fields when building settings', () => {
    const base = settings({ backend_future_field: { enabled: true } });
    const built = buildSettings(settingsToForm(base), base);

    expect(built.backend_future_field).toEqual({ enabled: true });
  });

  it('applies explicit patches after form values', () => {
    const base = settings();
    const built = buildSettings(settingsToForm(base), base, {
      steam_path: 'D:/Steam',
      custom_icon_enabled: true,
    });

    expect(built.steam_path).toBe('D:/Steam');
    expect(built.custom_icon_enabled).toBe(true);
  });

  it('reports a normalized fresh form as clean', () => {
    const base = settings({ show_store_nsfw: undefined });
    expect(isFormDirty(settingsToForm(base), base)).toBe(false);
  });

  it('detects persisted form changes', () => {
    const base = settings();
    const form = { ...settingsToForm(base), apiKey: 'changed' };
    expect(isFormDirty(form, base)).toBe(true);
  });

  it('excludes the immediately persisted OST acknowledgement from dirty state', () => {
    const base = settings({ ost_warning_acknowledged: false });
    const form = { ...settingsToForm(base), ostWarningAcknowledged: true };
    expect(isFormDirty(form, base)).toBe(false);
  });

  it('formats the library banner fields for the single-line input', () => {
    const form = settingsToForm(
      settings({
        library_banner_enabled: true,
        library_banner_appids: [4108000, 4656000],
        library_banner_steamid: '76561199876393402',
      })
    );

    expect(form.libraryBannerEnabled).toBe(true);
    expect(form.libraryBannerAppIds).toBe('4108000, 4656000');
    expect(form.libraryBannerSteamId).toBe('76561199876393402');
  });

  it('parses and normalizes the App ID list when saving', () => {
    const base = settings();
    const built = buildSettings(
      {
        ...settingsToForm(base),
        libraryBannerEnabled: true,
        libraryBannerAppIds: ' 4656000 ,4108000,4656000 ',
        libraryBannerSteamId: ' 76561199876393402 ',
      },
      base
    );

    expect(built.library_banner_enabled).toBe(true);
    expect(built.library_banner_appids).toEqual([4108000, 4656000]);
    expect(built.library_banner_steamid).toBe('76561199876393402');
  });

  it('defaults the library banner to off and clean', () => {
    const base = settings();
    const form = settingsToForm(base);

    expect(form.libraryBannerEnabled).toBe(false);
    expect(form.libraryBannerAppIds).toBe('');
    expect(isFormDirty(form, base)).toBe(false);
  });
});
