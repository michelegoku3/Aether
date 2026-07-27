import { invoke } from '@tauri-apps/api/core';

export interface AppSettings {
  hubcap_api_key: string;
  steam_path: string;
  active_library: string;
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
