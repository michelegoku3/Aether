export interface LuaToolsAuthStatus {
  signedIn: boolean;
  displayName: string | null;
  email: string | null;
}

export interface AppearanceAssets {
  themeExists: boolean;
  themeName: string | null;
  wallpaperExists: boolean;
  wallpaperName: string | null;
  iconExists: boolean;
  iconName: string | null;
  themesDir: string;
  wallpapersDir: string;
  iconsDir: string;
}

export type SteamCheckState = 'idle' | 'checking' | 'invalid';

export interface SteamCheckStatus {
  state: SteamCheckState;
  message: string;
}

export interface SettingsGuard {
  isDirty: () => boolean;
  save: () => Promise<boolean>;
  discard: () => void;
}
