import { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AppearanceAssets } from './types';
import type { SettingsPatch } from './settingsModel';

const EMPTY_ASSETS: AppearanceAssets = {
  themeExists: false, themeName: null, wallpaperExists: false, wallpaperName: null,
  iconExists: false, iconName: null, themesDir: '', wallpapersDir: '', iconsDir: '',
};
type StatusKind = 'info' | 'success' | 'error';

interface Params {
  persist: (patch: SettingsPatch) => Promise<void>;
  setTheme: (value: string) => void;
  setWallpaper: (value: string) => void;
  setIcon: (value: string) => void;
  setIconEnabled: (value: boolean) => void;
  refreshCustomCss: () => Promise<void>;
  showStatus: (text: string, kind: StatusKind) => void;
}

export function useAppearanceSettings(params: Params) {
  const [assets, setAssets] = useState<AppearanceAssets>(EMPTY_ASSETS);
  const [picking, setPicking] = useState<'theme' | 'wallpaper' | 'icon' | null>(null);
  const reload = useCallback(async () => {
    try { setAssets(await invoke<AppearanceAssets>('get_appearance_assets')); }
    catch (err) { console.warn('[settings] failed to load appearance assets:', err); }
  }, []);

  const pickTheme = useCallback(async () => {
    setPicking('theme');
    try {
      const fileName = await invoke<string>('pick_theme_file');
      params.setTheme(fileName); await params.persist({ theme_selected_file: fileName });
      await params.refreshCustomCss(); await reload(); params.showStatus(`Theme selected: ${fileName}`, 'success');
    } catch (err) { if (!String(err).includes('No file selected')) params.showStatus(`Failed to pick theme: ${err}`, 'error'); }
    finally { setPicking(null); }
  }, [params, reload]);

  const pickWallpaper = useCallback(async () => {
    setPicking('wallpaper');
    try {
      const fileName = await invoke<string>('pick_wallpaper_file');
      params.setWallpaper(fileName); await params.persist({ wallpaper_selected_file: fileName });
      await params.refreshCustomCss(); await reload(); params.showStatus(`Wallpaper selected: ${fileName}`, 'success');
    } catch (err) { if (!String(err).includes('No file selected')) params.showStatus(`Failed to pick wallpaper: ${err}`, 'error'); }
    finally { setPicking(null); }
  }, [params, reload]);

  const pickIcon = useCallback(async () => {
    setPicking('icon');
    try {
      const fileName = await invoke<string>('pick_icon_file');
      params.setIcon(fileName); await params.persist({ icon_selected_file: fileName, custom_icon_enabled: true });
      params.setIconEnabled(true); await invoke('apply_window_icon'); await reload();
      params.showStatus(`Icon selected: ${fileName}`, 'success');
    } catch (err) { if (!String(err).includes('No file selected')) params.showStatus(`Failed to pick icon: ${err}`, 'error'); }
    finally { setPicking(null); }
  }, [params, reload]);

  return { assets, picking, reload, pickTheme, pickWallpaper, pickIcon };
}
