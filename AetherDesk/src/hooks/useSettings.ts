import { invoke } from '@tauri-apps/api/core';

export interface AppSettings {
  hubcap_api_key: string;
  steam_path: string;
  active_library: string;
  /** Defaults to false on the backend: DLC-like rows are hidden from store search. */
  show_store_dlcs?: boolean;
  /** Defaults to TRUE on the backend: NSFW rows stay visible with a pink border. */
  show_store_nsfw?: boolean;
  /** Defaults to TRUE on the backend: delisted rows stay visible with a white border. */
  show_store_delisted?: boolean;
  /** Owned by the antivirus-exclusion flow; must be preserved verbatim on save. */
  antivirus_exclusion_done?: boolean;
  /** When true, AetherData/config/custom.css is injected as <style id="aether-custom-css">. Default false. */
  custom_css_enabled?: boolean;
  /** Ryuu API key (generator.ryuu.lol, 50/day, no verification endpoint) */
  ryuu_api_key?: string;
}

export const getSettings = async (): Promise<AppSettings> => {
  return invoke('get_settings');
};

export const requireSteamPath = async () => {
  const settings = await getSettings();
  if (!settings.steam_path || settings.steam_path.trim() === '') {
    throw new Error('Please specify the Steam path in Settings first.');
  }
  return settings.steam_path;
};
