import { invoke } from '@tauri-apps/api/core';

export interface AppSettings {
  hubcap_api_key: string;
  steam_path: string;
  /** Defaults to false on the backend: DLC-like rows are hidden from store search. */
  show_store_dlcs?: boolean;
  /** Defaults to TRUE on the backend: NSFW rows stay visible with a pink border. */
  show_store_nsfw?: boolean;
  /** Defaults to TRUE on the backend: delisted rows stay visible with a white border. */
  show_store_delisted?: boolean;
  /** Owned by the antivirus-exclusion flow; must be preserved verbatim on save. */
  antivirus_exclusion_done?: boolean;
  /** Owned by the OST first-enable warning flow; must be preserved verbatim on save. */
  ost_warning_acknowledged?: boolean;
  /** When true, AetherData/config/custom.css is injected as <style id="aether-custom-css">. Default false. */
  custom_css_enabled?: boolean;
  /** Ryuu API key (generator.ryuu.lol, 50/day, no verification endpoint) */
  ryuu_api_key?: string;
  /** Latest-version downloads comment setManifestid pins so Steam can update the game; OFF by default and requires an active Hubcap key. */
  download_games_with_updates_on?: boolean;
  /** Backend-owned one-time safe-default migration marker persisted in settings.json. */
  download_updates_default_off_migrated?: boolean;
  /** Show Store front games when no search is active. */
  show_store_front_games?: boolean;
  /** Alternate backdrop-focused card layout. */
  use_alternative_game_cards?: boolean;
  /** Enables WebView developer tools when supported by the build/runtime. */
  enable_webview_devtools?: boolean;
  /** Store front filter criterion. */
  store_front_filter?: string;
  /** Preferred Steam store currency for prices. */
  store_currency?: StoreCurrency | string;
  /** Personal wallpaper toggle. */
  personal_wallpaper_enabled?: boolean;
  /** Wallpaper opacity percentage (0..100). */
  personal_wallpaper_opacity?: number;
  /**
   * Library install-status filter:
   * `all` (default) | `installed` | `not_installed`.
   */
  library_install_filter?: 'all' | 'installed' | 'not_installed' | string;
  /** When true, AetherDesk also detects testing releases (tdesk, tdll prefixes). */
  enable_test_updates?: boolean;
  /** Custom game name displayed to friends on Steam (game_extra_info). */
  custom_game_name?: string;
}

/** Steam store currencies offered by the Settings selector (ISO 4217). The
 *  backend re-validates (unknown -> EUR), so this list only drives the UI. */
export type StoreCurrency =
  | 'eur' | 'usd' | 'gbp' | 'jpy' | 'ars' | 'brl' | 'cad' | 'aud' | 'chf'
  | 'cny' | 'krw' | 'inr' | 'mxn' | 'rub' | 'try' | 'pln' | 'sek' | 'nok'
  | 'dkk' | 'nzd' | 'sgd' | 'hkd' | 'twd' | 'thb' | 'myr' | 'idr' | 'php'
  | 'ils' | 'aed' | 'sar' | 'clp' | 'cop' | 'pen' | 'uah' | 'kzt' | 'vnd'
  | 'zar';

export const STORE_CURRENCIES: { code: StoreCurrency; label: string }[] = [
  { code: 'ars', label: 'Argentine Peso (AR$)' },
  { code: 'aud', label: 'Australian Dollar (A$)' },
  { code: 'brl', label: 'Brazilian Real (R$)' },
  { code: 'gbp', label: 'British Pound (\u00A3)' },
  { code: 'cad', label: 'Canadian Dollar (CA$)' },
  { code: 'clp', label: 'Chilean Peso (CL$)' },
  { code: 'cny', label: 'Chinese Yuan (CN\u00A5)' },
  { code: 'cop', label: 'Colombian Peso (CO$)' },
  { code: 'dkk', label: 'Danish Krone (kr)' },
  { code: 'eur', label: 'Euro (\u20AC)' },
  { code: 'hkd', label: 'Hong Kong Dollar (HK$)' },
  { code: 'inr', label: 'Indian Rupee (\u20B9)' },
  { code: 'idr', label: 'Indonesian Rupiah (Rp)' },
  { code: 'ils', label: 'Israeli Shekel (\u20AA)' },
  { code: 'kzt', label: 'Kazakhstani Tenge (\u20B8)' },
  { code: 'myr', label: 'Malaysian Ringgit (RM)' },
  { code: 'mxn', label: 'Mexican Peso (MX$)' },
  { code: 'nzd', label: 'New Zealand Dollar (NZ$)' },
  { code: 'nok', label: 'Norwegian Krone (kr)' },
  { code: 'pen', label: 'Peruvian Sol (S/)' },
  { code: 'php', label: 'Philippine Peso (\u20B1)' },
  { code: 'pln', label: 'Polish Zloty (z\u0142)' },
  { code: 'rub', label: 'Russian Ruble (\u20BD)' },
  { code: 'sar', label: 'Saudi Riyal (SAR)' },
  { code: 'sgd', label: 'Singapore Dollar (S$)' },
  { code: 'zar', label: 'South African Rand (R)' },
  { code: 'krw', label: 'South Korean Won (\u20A9)' },
  { code: 'sek', label: 'Swedish Krona (kr)' },
  { code: 'chf', label: 'Swiss Franc (CHF)' },
  { code: 'twd', label: 'Taiwan Dollar (NT$)' },
  { code: 'thb', label: 'Thai Baht (\u0E3F)' },
  { code: 'try', label: 'Turkish Lira (\u20BA)' },
  { code: 'aed', label: 'UAE Dirham (AED)' },
  { code: 'uah', label: 'Ukrainian Hryvnia (\u20B4)' },
  { code: 'usd', label: 'US Dollar ($)' },
  { code: 'vnd', label: 'Vietnamese Dong (\u20AB)' },
  { code: 'jpy', label: 'Yen (\u00A5)' },
];

export const isStoreCurrency = (value: unknown): value is StoreCurrency =>
  typeof value === 'string' &&
  STORE_CURRENCIES.some((c) => c.code === value);

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

/** Backend answer for read-only Steam-path validation (`check_steam_path`). */
export interface SteamPathCheck {
  valid: boolean;
  normalized: string;
  error: string | null;
}

/** Validates a Steam path against the backend (single source of truth). */
export const checkSteamPath = async (path: string): Promise<SteamPathCheck> => {
  return invoke('check_steam_path', { path });
};

/** True when the stored Steam path is a valid installation. Backend/IPC
 *  failures resolve to true: never nag about the Steam path when the check
 *  itself could not run. */
export const hasValidSteamPath = async (): Promise<boolean> => {
  try {
    const settings = await getSettings();
    const check = await checkSteamPath(settings.steam_path || '');
    return check.valid;
  } catch {
    return true;
  }
};
