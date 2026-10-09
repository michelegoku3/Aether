import { useCallback, useState } from 'react';
import { useCustomCss } from '../hooks/useCustomCss';
import { usePersonalWallpaper } from '../hooks/usePersonalWallpaper';
import { getSettings, type AppSettings } from '../hooks/useSettings';

const clampPercentage = (value: number) =>
  Math.max(0, Math.min(100, value));

interface Params {
  onSettingsApplied: (settings: AppSettings) => void | Promise<void>;
}

/** Owns live appearance previews and the settings generation observed by views. */
export function useAppearanceRuntime({ onSettingsApplied }: Params) {
  const [useAlternativeGameCards, setUseAlternativeGameCards] = useState(false);
  const [customCssEnabled, setCustomCssEnabled] = useState(false);
  const [personalWallpaperEnabled, setPersonalWallpaperEnabled] = useState(false);
  const [personalWallpaperOpacity, setPersonalWallpaperOpacity] = useState(20);
  const [alternativeCardsOpacity, setAlternativeCardsOpacity] = useState(100);
  const [alternativeCardsFade, setAlternativeCardsFade] = useState(50);
  const [wallpaperRevision, setWallpaperRevision] = useState(0);
  const [themeRevision, setThemeRevision] = useState(0);
  const [settingsRevision, setSettingsRevision] = useState(0);
  const [settingsReady, setSettingsReady] = useState(false);

  const applySettingsSnapshot = useCallback((settings: AppSettings) => {
    setUseAlternativeGameCards(Boolean(settings.use_alternative_game_cards));
    setCustomCssEnabled(Boolean(settings.custom_css_enabled));
    setPersonalWallpaperEnabled(Boolean(settings.personal_wallpaper_enabled));
    setPersonalWallpaperOpacity(clampPercentage(Number(settings.personal_wallpaper_opacity ?? 20)));
    setAlternativeCardsOpacity(clampPercentage(Number(settings.alternative_cards_opacity ?? 100)));
    setAlternativeCardsFade(clampPercentage(Number(settings.alternative_cards_fade ?? 50)));
  }, []);

  const refresh = useCallback(async (preloadedSettings?: AppSettings) => {
    try {
      const settings = preloadedSettings ?? await getSettings();
      applySettingsSnapshot(settings);
      setSettingsRevision((value) => value + 1);
      setWallpaperRevision((value) => value + 1);
      setThemeRevision((value) => value + 1);
      void onSettingsApplied(settings);
    } catch {
      setUseAlternativeGameCards(false);
      setCustomCssEnabled(false);
      setPersonalWallpaperEnabled(false);
      setPersonalWallpaperOpacity(20);
      setAlternativeCardsOpacity(100);
      setAlternativeCardsFade(50);
    } finally {
      setSettingsReady(true);
    }
  }, [applySettingsSnapshot, onSettingsApplied]);

  const changeCustomCss = useCallback((enabled: boolean) => {
    setCustomCssEnabled(enabled);
    setThemeRevision((value) => value + 1);
  }, []);

  const previewPersonalWallpaper = useCallback((enabled: boolean, opacity: number) => {
    setPersonalWallpaperEnabled(enabled);
    setPersonalWallpaperOpacity(clampPercentage(opacity));
  }, []);

  const previewAlternativeCards = useCallback((opacity: number, fade: number) => {
    setAlternativeCardsOpacity(clampPercentage(opacity));
    setAlternativeCardsFade(clampPercentage(fade));
  }, []);

  useCustomCss(customCssEnabled, themeRevision);
  usePersonalWallpaper(personalWallpaperEnabled, personalWallpaperOpacity, wallpaperRevision);

  return {
    useAlternativeGameCards,
    alternativeCardsOpacity,
    alternativeCardsFade,
    settingsRevision,
    settingsReady,
    refresh,
    changeCustomCss,
    previewPersonalWallpaper,
    previewAlternativeCards,
  };
}
