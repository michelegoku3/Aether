import { isStoreCurrency, type AppSettings, type StoreCurrency } from '../../hooks/useSettings';

export type SettingsPatch = Partial<AppSettings>;

export interface SettingsFormState {
  apiKey: string;
  steamPath: string;
  showStoreDlcs: boolean;
  showStoreNsfw: boolean;
  showStoreDelisted: boolean;
  downloadGamesWithUpdatesOn: boolean;
  workshopAutoDownloadContent: boolean;
  showStoreFrontGames: boolean;
  useAlternativeGameCards: boolean;
  enableWebviewDevtools: boolean;
  enableTestUpdates: boolean;
  customGameName: string;
  storeFrontFilter: string;
  storeCurrency: StoreCurrency;
  customCssEnabled: boolean;
  personalWallpaperEnabled: boolean;
  personalWallpaperOpacity: number;
  alternativeCardsOpacity: number;
  alternativeCardsFade: number;
  themeSelectedFile: string;
  wallpaperSelectedFile: string;
  customIconEnabled: boolean;
  iconSelectedFile: string;
  ryuuKey: string;
  ostWarningAcknowledged: boolean;
}

const clamp0to100 = (value: number) =>
  Math.max(0, Math.min(100, Number.isFinite(value) ? value : 0));

export const settingsToForm = (settings: AppSettings): SettingsFormState => ({
  apiKey: settings.hubcap_api_key || '',
  steamPath: settings.steam_path || '',
  showStoreDlcs: Boolean(settings.show_store_dlcs),
  showStoreNsfw: settings.show_store_nsfw !== false,
  showStoreDelisted: settings.show_store_delisted !== false,
  downloadGamesWithUpdatesOn: Boolean(settings.download_games_with_updates_on),
  workshopAutoDownloadContent: settings.workshop_auto_download_content !== false,
  showStoreFrontGames: settings.show_store_front_games !== false,
  useAlternativeGameCards: Boolean(settings.use_alternative_game_cards),
  enableWebviewDevtools: Boolean(settings.enable_webview_devtools),
  enableTestUpdates: Boolean(settings.enable_test_updates),
  customGameName: String(settings.custom_game_name || '').trim(),
  storeFrontFilter: String(settings.store_front_filter || 'upcoming'),
  storeCurrency: isStoreCurrency(settings.store_currency) ? settings.store_currency : 'eur',
  customCssEnabled: Boolean(settings.custom_css_enabled),
  personalWallpaperEnabled: Boolean(settings.personal_wallpaper_enabled),
  personalWallpaperOpacity: clamp0to100(Number(settings.personal_wallpaper_opacity ?? 20)),
  alternativeCardsOpacity: clamp0to100(Number(settings.alternative_cards_opacity ?? 100)),
  alternativeCardsFade: clamp0to100(Number(settings.alternative_cards_fade ?? 50)),
  themeSelectedFile: String(settings.theme_selected_file || ''),
  wallpaperSelectedFile: String(settings.wallpaper_selected_file || ''),
  customIconEnabled: Boolean(settings.custom_icon_enabled),
  iconSelectedFile: String(settings.icon_selected_file || ''),
  ryuuKey: String(settings.ryuu_api_key || ''),
  ostWarningAcknowledged: Boolean(settings.ost_warning_acknowledged),
});

export const buildSettings = (
  form: SettingsFormState,
  base: AppSettings,
  overrides: SettingsPatch = {},
): AppSettings => ({
  ...base,
  hubcap_api_key: form.apiKey,
  steam_path: form.steamPath,
  show_store_dlcs: form.showStoreDlcs,
  show_store_nsfw: form.showStoreNsfw,
  show_store_delisted: form.showStoreDelisted,
  custom_css_enabled: form.customCssEnabled,
  personal_wallpaper_enabled: form.personalWallpaperEnabled,
  personal_wallpaper_opacity: form.personalWallpaperOpacity,
  wallpaper_selected_file: form.wallpaperSelectedFile,
  theme_selected_file: form.themeSelectedFile,
  custom_icon_enabled: form.customIconEnabled,
  icon_selected_file: form.iconSelectedFile,
  alternative_cards_opacity: form.alternativeCardsOpacity,
  alternative_cards_fade: form.alternativeCardsFade,
  ryuu_api_key: form.ryuuKey,
  download_games_with_updates_on: form.downloadGamesWithUpdatesOn,
  workshop_auto_download_content: form.workshopAutoDownloadContent,
  show_store_front_games: form.showStoreFrontGames,
  use_alternative_game_cards: form.useAlternativeGameCards,
  enable_webview_devtools: form.enableWebviewDevtools,
  enable_test_updates: form.enableTestUpdates,
  custom_game_name: form.customGameName.trim(),
  store_front_filter: form.storeFrontFilter,
  store_currency: form.storeCurrency,
  ...overrides,
});

export const isFormDirty = (form: SettingsFormState, base: AppSettings): boolean => {
  const normalized = settingsToForm(base);
  return (Object.keys(form) as Array<keyof SettingsFormState>)
    .filter((key) => key !== 'ostWarningAcknowledged')
    .some((key) => form[key] !== normalized[key]);
};
