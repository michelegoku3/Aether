import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getSettings, checkSteamPath, STORE_CURRENCIES, isStoreCurrency, type SteamPathCheck, type StoreCurrency } from '../hooks/useSettings';
import { ClickablePath } from '../ui/ClickablePath';
import { EyeIcon, EyeOffIcon } from '../ui/icons';
import { OstWarningModal } from '../modals/OstWarningModal';
import { HubcapUpdateWarningModal } from '../modals/HubcapUpdateWarningModal';

interface SettingsViewProps {
  hubcapUsage: { usage: number; limit: number; hasKey: boolean };
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
  onRefreshCustomCss: () => Promise<void>;
  onCustomCssChange: (enabled: boolean) => void;
  onPreviewPersonalWallpaper: (enabled: boolean, opacity: number) => void;
  onPreviewAlternativeCards: (opacity: number, fade: number) => void;
  /** Called when a save is attempted without a valid Steam path (caller shows the warning modal). */
  onMissingSteamPath: () => void;
  /** Navigation guard slot: the view publishes isDirty/save/discard here so
   *  the App can prompt on tab switch / window close. */
  guardRef: { current: SettingsGuard | null };
}

interface LuaToolsAuthStatus {
  signedIn: boolean;
  displayName: string | null;
  email: string | null;
}

interface AppearanceAssets {
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

// No 'valid' state: valid paths show nothing by design, only checking/invalid.
type SteamCheckState = 'idle' | 'checking' | 'invalid';

interface SteamCheckStatus {
  state: SteamCheckState;
  message: string;
}

/** Navigation-guard API published by SettingsView for tab-switch and
 *  window-close prompts (owned by App, which holds the modal). */
export interface SettingsGuard {
  /** True when the form differs from the last saved/applied state. */
  isDirty: () => boolean;
  /** Saves; resolves true when the save succeeded. */
  save: () => Promise<boolean>;
  /** Reverts the form (and live previews) to the last saved/applied state. */
  discard: () => void;
}

const clamp0to100 = (value: number) => Math.max(0, Math.min(100, Number.isFinite(value) ? value : 0));

export const SettingsView = ({ hubcapUsage, onRefreshUsage, onRefreshCustomCss, onCustomCssChange, onPreviewPersonalWallpaper, onPreviewAlternativeCards, onMissingSteamPath, guardRef }: SettingsViewProps) => {
  const [apiKey, setApiKey] = useState('');
  const [showApiKey, setShowApiKey] = useState(false);
  const [steamPath, setSteamPath] = useState('');
  /** Live validation status for the Steam path field (debounced backend check). */
  const [steamCheck, setSteamCheck] = useState<SteamCheckStatus>({ state: 'idle', message: '' });
  const steamCheckRequestId = useRef(0);
  const [showStoreDlcs, setShowStoreDlcs] = useState(false);
  const [showStoreNsfw, setShowStoreNsfw] = useState(true);
  const [showStoreDelisted, setShowStoreDelisted] = useState(true);
  const [downloadGamesWithUpdatesOn, setDownloadGamesWithUpdatesOn] = useState(false);
  const [showHubcapUpdateWarning, setShowHubcapUpdateWarning] = useState(false);
  const [showStoreFrontGames, setShowStoreFrontGames] = useState(true);
  const [useAlternativeGameCards, setUseAlternativeGameCards] = useState(false);
  const [enableWebviewDevtools, setEnableWebviewDevtools] = useState(false);
  const [enableTestUpdates, setEnableTestUpdates] = useState(false);
  // [network] use_ost_source in aethercore.toml: OST pattern source opt-in
  // (default OFF), applies immediately like the presence default_mode toggle.
  const [useOstSource, setUseOstSource] = useState(false);
  // First-enable warning: shown once when flipping the OST switch ON before
  // the user has pressed "I understand". Persisted in settings.json.
  const [ostWarningAcknowledged, setOstWarningAcknowledged] = useState(false);
  const [showOstWarning, setShowOstWarning] = useState(false);
  // [manifest_cache] restore_on_startup in aethercore.toml: refill
  // Steam\depotcache from the manifest backups on every Steam start
  // (default ON), applies on the next Steam start.
  const [manifestRestoreOnStartup, setManifestRestoreOnStartup] = useState(true);
  // [presence] default_mode in aethercore.toml (docs/05 §12): live nel file
  // della DLL, NON nelle Desk settings — si applica subito, senza Save.
  const [presenceDefaultShowOnline, setPresenceDefaultShowOnline] = useState(true);
  const [customGameName, setCustomGameName] = useState('');
  const [storeFrontFilter, setStoreFrontFilter] = useState('upcoming');
  const [customCssEnabled, setCustomCssEnabled] = useState(false);
  const [personalWallpaperEnabled, setPersonalWallpaperEnabled] = useState(false);
  const [personalWallpaperOpacity, setPersonalWallpaperOpacity] = useState(20);
  const [alternativeCardsOpacity, setAlternativeCardsOpacity] = useState(100);
  const [alternativeCardsFade, setAlternativeCardsFade] = useState(50);
  const [themeSelectedFile, setThemeSelectedFile] = useState('');
  const [wallpaperSelectedFile, setWallpaperSelectedFile] = useState('');
  const [customIconEnabled, setCustomIconEnabled] = useState(false);
  const [iconSelectedFile, setIconSelectedFile] = useState('');
  const [ryuuKey, setRyuuKey] = useState('');
  const [showRyuuKey, setShowRyuuKey] = useState(false);
  const [storeCurrency, setStoreCurrency] = useState<StoreCurrency>('eur');
  const [luaToolsAuth, setLuaToolsAuth] = useState<LuaToolsAuthStatus>({ signedIn: false, displayName: null, email: null });
  const [isLuaToolsAuthBusy, setIsLuaToolsAuthBusy] = useState(false);
  const [isLuaToolsOAuthBusy, setIsLuaToolsOAuthBusy] = useState(false);
  const [showLuaToolsLoginModal, setShowLuaToolsLoginModal] = useState(false);
  const [luaToolsLoginMode, setLuaToolsLoginMode] = useState<'choice' | 'code'>('choice');
  const [luaToolsLoginCode, setLuaToolsLoginCode] = useState('');
  const [luaToolsLoginError, setLuaToolsLoginError] = useState('');

  // Appearance assets availability: when no theme/wallpaper file exists, the
  // corresponding switch must stay disabled (cannot be enabled).
  const [appearanceAssets, setAppearanceAssets] = useState<AppearanceAssets>({
    themeExists: false,
    themeName: null,
    wallpaperExists: false,
    wallpaperName: null,
    iconExists: false,
    iconName: null,
    themesDir: '',
    wallpapersDir: '',
    iconsDir: '',
  });
  const [isPicking, setIsPicking] = useState<'theme' | 'wallpaper' | 'icon' | null>(null);

  // Raw settings as loaded from the backend. Saving always spreads this object
  // back, so fields owned by other flows (e.g. `antivirus_exclusion_done`) are
  // never silently reset by a settings save.
  const [rawSettings, setRawSettings] = useState<Record<string, any>>({});

  const [statusMsg, setStatusMsg] = useState({ text: '', type: 'info' });

  const showStatus = (text: string, type: 'info' | 'success' | 'error') => {
    setStatusMsg({ text, type });
    setTimeout(() => setStatusMsg({ text: '', type: 'info' }), 6000);
  };

  const loadAppearanceAssets = async () => {
    try {
      const assets: AppearanceAssets = await invoke('get_appearance_assets');
      setAppearanceAssets(assets);
    } catch (err) {
      console.warn('[settings] failed to load appearance assets:', err);
    }
  };

  /** Applies a settings object to every piece of React state. Single mapping
   *  shared by initial load and Reset, so the two can never drift apart. */
  const applySettingsToState = (settings: Record<string, any>) => {
    setRawSettings(settings);
    setApiKey(settings.hubcap_api_key || '');
    setSteamPath(settings.steam_path || '');
    setShowStoreDlcs(Boolean(settings.show_store_dlcs));
    // These two default to enabled: only an explicit `false` turns them off.
    setShowStoreNsfw(settings.show_store_nsfw !== false);
    setShowStoreDelisted(settings.show_store_delisted !== false);
    setDownloadGamesWithUpdatesOn(Boolean(settings.download_games_with_updates_on));
    setShowStoreFrontGames(settings.show_store_front_games !== false);
    setUseAlternativeGameCards(Boolean(settings.use_alternative_game_cards));
    setEnableWebviewDevtools(Boolean(settings.enable_webview_devtools));
    setEnableTestUpdates(Boolean(settings.enable_test_updates));
    // Trimmed like buildCurrentSettings does on save, so the dirty-compare
    // never flags a freshly loaded form over surrounding whitespace.
    setCustomGameName((settings.custom_game_name || '').trim());
    setStoreFrontFilter(settings.store_front_filter || 'upcoming');
    setCustomCssEnabled(Boolean(settings.custom_css_enabled));
    setPersonalWallpaperEnabled(Boolean(settings.personal_wallpaper_enabled));
    setPersonalWallpaperOpacity(clamp0to100(Number(settings.personal_wallpaper_opacity ?? 20)));
    setAlternativeCardsOpacity(clamp0to100(Number(settings.alternative_cards_opacity ?? 100)));
    setAlternativeCardsFade(clamp0to100(Number(settings.alternative_cards_fade ?? 50)));
    setThemeSelectedFile(settings.theme_selected_file || '');
    setWallpaperSelectedFile(settings.wallpaper_selected_file || '');
    setCustomIconEnabled(Boolean(settings.custom_icon_enabled));
    setIconSelectedFile(settings.icon_selected_file || '');
    setRyuuKey(settings.ryuu_api_key || '');
    setOstWarningAcknowledged(Boolean(settings.ost_warning_acknowledged));
    setStoreCurrency(isStoreCurrency(settings.store_currency) ? settings.store_currency : 'eur');
  };

  // Load settings from the backend when the component mounts.
  // NOTE: the component stays mounted across tab switches (see MainContent),
  // so this runs once: edits are preserved, and every save merges over a
  // freshly loaded snapshot (see loadFreshBase) instead of stale state.
  useEffect(() => {
    const loadSettings = async () => {
      try {
        const settings: any = await invoke('get_settings');
        if (settings) {
          applySettingsToState(settings);
        }
      } catch (err: any) {
        showStatus(`Error loading settings: ${err}`, 'error');
      }
    };
    loadSettings();
    loadAppearanceAssets();
    // Presence default policy lives in aethercore.toml (not Desk settings).
    invoke<boolean>('get_presence_default_mode')
      .then(setPresenceDefaultShowOnline)
      .catch((err) => console.warn('[settings] failed to load presence default mode:', err));
    // OST pattern source opt-in lives in aethercore.toml too (default OFF).
    invoke<boolean>('get_ost_source_enabled')
      .then(setUseOstSource)
      .catch((err) => console.warn('[settings] failed to load OST source state:', err));
    // Manifest restore on Steam startup lives in aethercore.toml too (default ON).
    invoke<boolean>('get_manifest_restore_enabled')
      .then(setManifestRestoreOnStartup)
      .catch((err) => console.warn('[settings] failed to load manifest restore state:', err));
    invoke<LuaToolsAuthStatus>('get_luatools_auth_status')
      .then(setLuaToolsAuth)
      .catch((err) => console.warn('[settings] failed to load LuaTools auth status:', err));
    onRefreshUsage();
  }, []);

  /** Fresh settings snapshot for saves: merging UI state over a fresh read
   *  (instead of the mount-time `rawSettings`) guarantees concurrent saves
   *  from other flows are never regressed. Falls back to `rawSettings`. */
  const loadFreshBase = async (): Promise<Record<string, any>> => {
    try {
      return await getSettings();
    } catch {
      return rawSettings;
    }
  };

  /** Constructs the full current settings object from React state variables,
   *  merging over a base snapshot. The base should be freshly loaded (see
   *  `loadFreshBase`): merging over the mount-time `rawSettings` would
   *  regress concurrent saves from other flows (e.g. the Library filter). */
  const buildCurrentSettings = (overrides: Record<string, any> = {}, base: Record<string, any> = rawSettings) => ({
    ...base,
    hubcap_api_key: apiKey,
    steam_path: steamPath,
    show_store_dlcs: showStoreDlcs,
    show_store_nsfw: showStoreNsfw,
    show_store_delisted: showStoreDelisted,
    custom_css_enabled: customCssEnabled,
    personal_wallpaper_enabled: personalWallpaperEnabled,
    personal_wallpaper_opacity: personalWallpaperOpacity,
    wallpaper_selected_file: wallpaperSelectedFile,
    theme_selected_file: themeSelectedFile,
    custom_icon_enabled: customIconEnabled,
    icon_selected_file: iconSelectedFile,
    alternative_cards_opacity: alternativeCardsOpacity,
    alternative_cards_fade: alternativeCardsFade,
    ryuu_api_key: ryuuKey,
    download_games_with_updates_on: downloadGamesWithUpdatesOn,
    show_store_front_games: showStoreFrontGames,
    use_alternative_game_cards: useAlternativeGameCards,
    enable_webview_devtools: enableWebviewDevtools,
    enable_test_updates: enableTestUpdates,
    custom_game_name: customGameName.trim(),
    store_front_filter: storeFrontFilter,
    store_currency: storeCurrency,
    ...overrides,
  });

  /** Saves the form; resolves true when the save succeeded. Shared by the
   *  Save button and the unsaved-changes navigation guard. */
  const doSave = async (): Promise<boolean> => {
    // API key validation must never block the whole save: when the key is
    // invalid the rest of the settings still persist, the bad key field is
    // cleared and a warning tells the user what happened. Only a genuinely
    // invalid key is wiped — if validation itself cannot complete (e.g. the
    // service is unreachable) the key is kept and the save still goes through.
    let invalidApiKey = false;
    let apiKeyWarning = '';
    let hubcapKeyValid = false;
    if (apiKey.trim()) {
      showStatus('Validating API key...', 'info');
      try {
        const isValid: any = await invoke('validate_hubcap_key', { apiKey: apiKey.trim() });
        hubcapKeyValid = Boolean(isValid);
        if (!isValid) {
          invalidApiKey = true;
          apiKeyWarning = 'The Hubcap API key is invalid and was cleared; the rest of your settings were saved.';
        }
      } catch (err: any) {
        apiKeyWarning = `Hubcap API key could not be validated (${err}); the rest of your settings were saved anyway.`;
      }
    }

    if (downloadGamesWithUpdatesOn && !hubcapKeyValid) {
      // Never persist an enabled update policy without a positively validated
      // Hubcap key, including when an older settings file had the toggle ON.
      setDownloadGamesWithUpdatesOn(false);
      setShowHubcapUpdateWarning(true);
      return false;
    }

    // A save without a valid Steam path is rejected by the backend anyway:
    // pop the OST-style warning instead of failing into a toast. (If the
    // check itself fails, fall through so the save surfaces the real error.)
    try {
      if (!(await checkSteamPath(steamPath)).valid) {
        onMissingSteamPath();
        return false;
      }
    } catch { /* fall through to the save below */ }
    // Persist settings, merging over a fresh snapshot (never over the
    // possibly stale mount-time state).
    try {
      showStatus('Saving settings...', 'info');
      // If the user just enabled Custom CSS, ensure the file exists before
      // saving so the next editor open does not find an empty folder.
      if (customCssEnabled || personalWallpaperEnabled) {
        try { await invoke('ensure_custom_css'); } catch {}
      }
      const newSettings = buildCurrentSettings(
        invalidApiKey ? { hubcap_api_key: '' } : {},
        await loadFreshBase()
      );
      await invoke('save_settings', { settings: newSettings });
      setRawSettings(newSettings);
      if (invalidApiKey) {
        setApiKey(''); // clear the wrong key from the field
      }
      showStatus(
        apiKeyWarning || 'Settings saved successfully!',
        apiKeyWarning ? 'error' : 'success'
      );
      onRefreshUsage(invalidApiKey ? '' : apiKey);
      onRefreshCustomCss();
      loadAppearanceAssets();
      return true;
    } catch (err: any) {
      const message = String(err);
      if (message.includes('HUBCAP_KEY_REQUIRED_FOR_UPDATES')) {
        setDownloadGamesWithUpdatesOn(false);
        setShowHubcapUpdateWarning(true);
      }
      showStatus(`Error during save: ${message}`, 'error');
      return false;
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    await doSave();
  };

  /** Persists a freshly picked theme/wallpaper file immediately (the selection
   *  is part of settings) and refreshes the live preview. Using buildCurrentSettings
   *  and updating rawSettings guarantees we never revert toggles or wipe out
   *  an un-saved API key. */
  const persistAppearanceSelection = async (patch: Record<string, any>) => {
    const newSettings = buildCurrentSettings(patch, await loadFreshBase());
    await invoke('save_settings', { settings: newSettings });
    setRawSettings(newSettings);
  };

  /** True when the form differs from the last saved/applied snapshot
   *  (rawSettings). Mirrors applySettingsToState normalization so freshly
   *  loaded/saved forms never compare dirty. Live-only fields (OST source,
   *  presence default) are excluded: they apply immediately and are never
   *  "unsaved". */
  const isSettingsDirty = (): boolean => {
    const s = rawSettings;
    return (
      apiKey !== (s.hubcap_api_key || '') ||
      steamPath !== (s.steam_path || '') ||
      showStoreDlcs !== Boolean(s.show_store_dlcs) ||
      showStoreNsfw !== (s.show_store_nsfw !== false) ||
      showStoreDelisted !== (s.show_store_delisted !== false) ||
      downloadGamesWithUpdatesOn !== Boolean(s.download_games_with_updates_on) ||
      showStoreFrontGames !== (s.show_store_front_games !== false) ||
      useAlternativeGameCards !== Boolean(s.use_alternative_game_cards) ||
      enableWebviewDevtools !== Boolean(s.enable_webview_devtools) ||
      enableTestUpdates !== Boolean(s.enable_test_updates) ||
      customGameName !== ((s.custom_game_name || '') as string).trim() ||
      storeFrontFilter !== (s.store_front_filter || 'upcoming') ||
      storeCurrency !== (isStoreCurrency(s.store_currency) ? s.store_currency : 'eur') ||
      customCssEnabled !== Boolean(s.custom_css_enabled) ||
      personalWallpaperEnabled !== Boolean(s.personal_wallpaper_enabled) ||
      personalWallpaperOpacity !== clamp0to100(Number(s.personal_wallpaper_opacity ?? 20)) ||
      alternativeCardsOpacity !== clamp0to100(Number(s.alternative_cards_opacity ?? 100)) ||
      alternativeCardsFade !== clamp0to100(Number(s.alternative_cards_fade ?? 50)) ||
      themeSelectedFile !== (s.theme_selected_file || '') ||
      wallpaperSelectedFile !== (s.wallpaper_selected_file || '') ||
      customIconEnabled !== Boolean(s.custom_icon_enabled) ||
      iconSelectedFile !== (s.icon_selected_file || '') ||
      ryuuKey !== (s.ryuu_api_key || '')
    );
  };

  /** Reverts the form to the last saved/applied snapshot, including live
   *  previews (custom CSS, wallpaper, alternative cards) which apply
   *  instantly and would otherwise stay at the discarded values. */
  const discardChanges = (): void => {
    applySettingsToState(rawSettings);
    onCustomCssChange(Boolean(rawSettings.custom_css_enabled));
    onPreviewPersonalWallpaper(
      Boolean(rawSettings.personal_wallpaper_enabled),
      clamp0to100(Number(rawSettings.personal_wallpaper_opacity ?? 20))
    );
    onPreviewAlternativeCards(
      clamp0to100(Number(rawSettings.alternative_cards_opacity ?? 100)),
      clamp0to100(Number(rawSettings.alternative_cards_fade ?? 50))
    );
  };

  // Publish the navigation guard (fresh closures every render; cleared on
  // unmount so the App never calls into a dead view).
  guardRef.current = { isDirty: isSettingsDirty, save: doSave, discard: discardChanges };
  useEffect(() => () => { guardRef.current = null; }, [guardRef]);

  /** Live Steam-path validation (debounced, read-only backend check). A
   *  monotonic request id drops stale responses when the user keeps typing. */
  useEffect(() => {
    const requestId = ++steamCheckRequestId.current;
    setSteamCheck({ state: 'checking', message: 'Checking Steam path...' });
    const timer = window.setTimeout(() => {
      void (async () => {
        let next: SteamCheckStatus;
        try {
          const result: SteamPathCheck = await checkSteamPath(steamPath);
          next = result.valid
            ? { state: 'idle', message: '' }
            : { state: 'invalid', message: result.error || 'Invalid Steam path.' };
        } catch {
          next = { state: 'invalid', message: 'Could not validate the Steam path.' };
        }
        if (requestId === steamCheckRequestId.current) {
          setSteamCheck(next);
        }
      })();
    }, 400);
    return () => window.clearTimeout(timer);
  }, [steamPath]);

  /** Persists ONLY the Steam path over freshly loaded settings (every other
   *  field preserved, other unsaved UI edits left dirty in the form). Used by
   *  Browse / Auto Detect, which are explicit enough to save immediately --
   *  but only after the backend confirms the path is a valid installation. */
  const saveSteamPathNow = async (path: string, actionLabel: string) => {
    setSteamPath(path);
    let check: SteamPathCheck;
    try {
      check = await checkSteamPath(path);
    } catch {
      check = { valid: false, normalized: path, error: 'Could not validate the Steam path.' };
    }
    if (!check.valid) {
      setSteamCheck({ state: 'invalid', message: check.error || 'Invalid Steam path.' });
      showStatus(`${actionLabel}, but it is not a valid Steam installation: ${check.error || path}`, 'error');
      return;
    }
    try {
      const merged = { ...(await loadFreshBase()), steam_path: check.normalized };
      await invoke('save_settings', { settings: merged });
      setRawSettings(merged);
      setSteamPath(check.normalized);
      setSteamCheck({ state: 'idle', message: '' });
      showStatus(`${actionLabel}: ${check.normalized}`, 'success');
      onRefreshCustomCss();
    } catch (err: any) {
      showStatus(`Failed to save Steam path: ${err}`, 'error');
    }
  };

  const handleBrowseSteamFolder = async () => {
    try {
      const picked: string | null = await invoke('pick_steam_folder');
      if (!picked) return; // dialog cancelled
      await saveSteamPathNow(picked, 'Steam folder selected');
    } catch (err: any) {
      showStatus(`Failed to open folder picker: ${err}`, 'error');
    }
  };

  const handleDetectSteamPath = async () => {
    try {
      const detected: string | null = await invoke('detect_steam_path');
      if (!detected) {
        showStatus('No Steam installation detected. Use Browse to select it manually.', 'error');
        return;
      }
      await saveSteamPathNow(detected, 'Steam auto-detected');
    } catch (err: any) {
      showStatus(`Steam auto-detection failed: ${err}`, 'error');
    }
  };

  const handlePickTheme = async () => {
    setIsPicking('theme');
    try {
      const fileName: string = await invoke('pick_theme_file');
      setThemeSelectedFile(fileName);
      await persistAppearanceSelection({ theme_selected_file: fileName });
      await onRefreshCustomCss();
      await loadAppearanceAssets();
      showStatus(`Theme selected: ${fileName}`, 'success');
    } catch (err: any) {
      // "No file selected" is a normal cancellation, not an error.
      if (String(err).includes('No file selected')) return;
      showStatus(`Failed to pick theme: ${err}`, 'error');
    } finally {
      setIsPicking(null);
    }
  };

  const handlePickIcon = async () => {
    setIsPicking('icon');
    try {
      const fileName: string = await invoke('pick_icon_file');
      setIconSelectedFile(fileName);
      await persistAppearanceSelection({ icon_selected_file: fileName, custom_icon_enabled: true });
      // Picking an icon enables it on disk: reflect it in the toggle too.
      setCustomIconEnabled(true);
      await invoke('apply_window_icon');
      await loadAppearanceAssets();
      showStatus(`Icon selected: ${fileName}`, 'success');
    } catch (err: any) {
      if (String(err).includes('No file selected')) return;
      showStatus(`Failed to pick icon: ${err}`, 'error');
    } finally {
      setIsPicking(null);
    }
  };

  const handlePickWallpaper = async () => {
    setIsPicking('wallpaper');
    try {
      const fileName: string = await invoke('pick_wallpaper_file');
      setWallpaperSelectedFile(fileName);
      await persistAppearanceSelection({ wallpaper_selected_file: fileName });
      await onRefreshCustomCss();
      await loadAppearanceAssets();
      showStatus(`Wallpaper selected: ${fileName}`, 'success');
    } catch (err: any) {
      if (String(err).includes('No file selected')) return;
      showStatus(`Failed to pick wallpaper: ${err}`, 'error');
    } finally {
      setIsPicking(null);
    }
  };

  const openLuaToolsLogin = () => {
    setLuaToolsLoginMode('choice');
    setLuaToolsLoginCode('');
    setLuaToolsLoginError('');
    setShowLuaToolsLoginModal(true);
  };

  const closeLuaToolsLogin = () => {
    if (isLuaToolsAuthBusy) return;
    setShowLuaToolsLoginModal(false);
    setLuaToolsLoginMode('choice');
    setLuaToolsLoginCode('');
    setLuaToolsLoginError('');
  };

  const handleLuaToolsSignIn = async () => {
    if (isLuaToolsAuthBusy) return;
    setShowLuaToolsLoginModal(false);
    setIsLuaToolsAuthBusy(true);
    setIsLuaToolsOAuthBusy(true);
    showStatus('Complete the LuaTools OAuth sign-in in Discord or your browser...', 'info');
    try {
      const auth = await invoke<LuaToolsAuthStatus>('sign_in_luatools');
      setLuaToolsAuth(auth);
      showStatus(`LuaTools connected${auth.displayName ? ` as ${auth.displayName}` : ''}.`, 'success');
    } catch (err: any) {
      if (!String(err).toLowerCase().includes('cancelled')) {
        showStatus(`LuaTools sign-in failed: ${err}`, 'error');
      }
    } finally {
      setIsLuaToolsOAuthBusy(false);
      setIsLuaToolsAuthBusy(false);
    }
  };

  const cancelLuaToolsOAuth = async () => {
    try {
      await invoke('cancel_luatools_sign_in');
      showStatus('LuaTools sign-in cancelled.', 'info');
    } catch (err: any) {
      showStatus(`Could not cancel LuaTools sign-in: ${err}`, 'error');
    } finally {
      setIsLuaToolsOAuthBusy(false);
      setIsLuaToolsAuthBusy(false);
    }
  };

  const handleLuaToolsCodeSignIn = async () => {
    if (isLuaToolsAuthBusy) return;
    const code = luaToolsLoginCode.trim().toUpperCase();
    if (code.length !== 6) {
      setLuaToolsLoginError('Enter the 6-character code generated by @Luie.');
      return;
    }
    setIsLuaToolsAuthBusy(true);
    setLuaToolsLoginError('');
    try {
      const auth = await invoke<LuaToolsAuthStatus>('sign_in_luatools_with_code', { code });
      setLuaToolsAuth(auth);
      setShowLuaToolsLoginModal(false);
      setLuaToolsLoginMode('choice');
      setLuaToolsLoginCode('');
      showStatus(`LuaTools connected privately${auth.displayName ? ` as ${auth.displayName}` : ''}.`, 'success');
    } catch (err: any) {
      setLuaToolsLoginError(String(err));
    } finally {
      setIsLuaToolsAuthBusy(false);
    }
  };

  const handleLuaToolsSignOut = async () => {
    if (isLuaToolsAuthBusy) return;
    setIsLuaToolsAuthBusy(true);
    try {
      await invoke('sign_out_luatools');
      setLuaToolsAuth({ signedIn: false, displayName: null, email: null });
      showStatus('LuaTools disconnected.', 'success');
    } catch (err: any) {
      showStatus(`LuaTools sign-out failed: ${err}`, 'error');
    } finally {
      setIsLuaToolsAuthBusy(false);
    }
  };

  /** Latest-version downloads must use authenticated Hubcap manifest access.
   * Validate on every enable attempt; there is deliberately no acknowledgement
   * flag because the warning must reappear whenever the key is absent/invalid. */
  const handleDownloadGamesWithUpdatesChange = async (enabled: boolean) => {
    if (!enabled) {
      setDownloadGamesWithUpdatesOn(false);
      return;
    }

    const key = apiKey.trim();
    if (!key) {
      setShowHubcapUpdateWarning(true);
      return;
    }

    try {
      const valid = await invoke<boolean>('validate_hubcap_key', { apiKey: key });
      if (!valid) {
        setShowHubcapUpdateWarning(true);
        return;
      }
      setDownloadGamesWithUpdatesOn(true);
      showStatus('Hubcap key validated. Latest downloads may enable Steam updates.', 'success');
    } catch {
      // A key that cannot be validated is not treated as active. This keeps
      // the toggle off instead of silently enabling a broken update path.
      setShowHubcapUpdateWarning(true);
    }
  };

  /** "I understand" on the OST first-enable warning: persist the ack, then
   *  enable the source. The switch flips only on success, so any failure
   *  leaves it OFF and the popup will reappear on the next attempt. */
  const handleOstWarningConfirm = async () => {
    try {
      try { await invoke('acknowledge_ost_warning'); } catch {}
      await invoke('set_ost_source_enabled', { enabled: true });
      setOstWarningAcknowledged(true);
      setUseOstSource(true);
      setShowOstWarning(false);
      // Keep rawSettings in sync so a later "Save Settings" (which spreads
      // rawSettings via buildCurrentSettings) does not regress the ack.
      setRawSettings((prev) => ({ ...prev, ost_warning_acknowledged: true }));
    } catch (err: any) {
      setShowOstWarning(false);
      showStatus(`Failed to set OST pattern source: ${err}`, 'error');
    }
  };

  const appearancePickBtn = (label: string, onClick: () => void, disabled: boolean, busy: boolean) => (
    <button
      type="button"
      className="appearance-pick-btn"
      onClick={onClick}
      disabled={disabled || busy}
    >
      {busy ? '...' : label}
    </button>
  );

  return (
    <div className="settings-view">
      <div className="settings-header">
        <h1 className="settings-title">Settings</h1>
        <p className="settings-subtitle">Manage system configurations, API keys, Steam injection paths, and other settings.</p>
      </div>

      <div className="settings-separator"></div>

      {statusMsg.text && (
        <div className={`settings-alert ${statusMsg.type}`}>
          {statusMsg.text}
        </div>
      )}

      <form onSubmit={handleSave} className="settings-form">
        {/* AETHER section: dev utilities, titled like the other groups. */}
        <div className="settings-group">
          <label className="settings-label">Aether</label>
          <div className="settings-toggle-row" title="Open WebView developer tools. They open only when you switch this ON manually.">
            <span className="settings-toggle-text">Enable WebView devtools</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={enableWebviewDevtools}
                onChange={async (e) => {
                  const next = e.target.checked;
                  setEnableWebviewDevtools(next);
                  // Open the devtools only on an explicit user action, never
                  // automatically on startup/save.
                  if (next) {
                    try { await invoke('open_webview_devtools'); } catch (err) { console.warn('Unable to open WebView devtools:', err); }
                  }
                }}
              />
              <span></span>
            </label>
          </div>

          <div className="settings-toggle-row" title="When ON, AetherDesk also detects testing releases (tdesk-*/tdll-*) and gives them priority. Test updates are shown with a red dot. Keep OFF unless you are testing a build.">
            <span className="settings-toggle-text">Enable test updates</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={enableTestUpdates}
                onChange={(e) => setEnableTestUpdates(e.target.checked)}
              />
              <span></span>
            </label>
          </div>

          <div
            className="settings-toggle-row"
            title="Opt-in fallback to the OpenSteamTool pattern source (OpenSteam001/steam-monitor) for Steam build patterns. OFF by default: only MigoReleases and KoriaPolis are used. Applies immediately (no Save, no Steam restart); takes effect on the next pattern download."
          >
            <span className="settings-toggle-text">Use OST pattern source</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={useOstSource}
                onChange={async (e) => {
                  const next = e.target.checked;
                  // First enable requires the warning popup: opening it
                  // changes nothing, so dismissing (X/ESC/overlay) leaves the
                  // switch OFF by construction. Only "I understand" enables.
                  if (next && !ostWarningAcknowledged) {
                    setShowOstWarning(true);
                    return;
                  }
                  const previous = !next;
                  // Applica subito (il toggle scrive aethercore.toml, non le
                  // Desk settings); rollback ottimistico in caso di errore.
                  setUseOstSource(next);
                  try {
                    await invoke('set_ost_source_enabled', { enabled: next });
                  } catch (err: any) {
                    setUseOstSource(previous);
                    showStatus(`Failed to set OST pattern source: ${err}`, 'error');
                  }
                }}
              />
              <span></span>
            </label>
          </div>

          <div
            className="settings-toggle-row"
            title="When ON (default), AetherCore copies every backed-up .manifest (AetherData\backup\<app_id>\lua) that is missing from Steam\depotcache back into place on each Steam start. Uninstalling a game wipes its manifests, and Steam no longer serves manifests without authentication — this keeps them always available. Applies immediately (no Save); takes effect on the next Steam start."
          >
            <span className="settings-toggle-text">Restore manifests on Steam startup</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={manifestRestoreOnStartup}
                onChange={async (e) => {
                  const next = e.target.checked;
                  const previous = !next;
                  // Applica subito (il toggle scrive aethercore.toml, non le
                  // Desk settings); rollback ottimistico in caso di errore.
                  setManifestRestoreOnStartup(next);
                  try {
                    await invoke('set_manifest_restore_enabled', { enabled: next });
                  } catch (err: any) {
                    setManifestRestoreOnStartup(previous);
                    showStatus(`Failed to set manifest restore: ${err}`, 'error');
                  }
                }}
              />
              <span></span>
            </label>
          </div>

          <div
            className="settings-toggle-row"
            title="Controls the [presence] default_mode in aethercore.toml and applies immediately (no Save, no Steam restart). ON: friends see what you play for EVERY game you launch — unless the game has a per-game mode in the ONLINE popup (Show / Online Aether / excluded). OFF: nothing is shown by default; you hand-pick games per-game. With a Custom game display name set, every game is treated as showonline regardless of this policy."
          >
            <span className="settings-toggle-text">Show every game to friends by default</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={presenceDefaultShowOnline}
                onChange={async (e) => {
                  const next = e.target.checked;
                  const previous = !next;
                  // Applica subito (il toggle scrive aethercore.toml, non le
                  // Desk settings); rollback ottimistico in caso di errore.
                  setPresenceDefaultShowOnline(next);
                  try {
                    await invoke('set_presence_default_mode', { showonline: next });
                  } catch (err: any) {
                    setPresenceDefaultShowOnline(previous);
                    showStatus(`Failed to set presence default mode: ${err}`, 'error');
                  }
                }}
              />
              <span></span>
            </label>
          </div>

          <div className="settings-toggle-row settings-field-column">
            <span className="settings-toggle-text">Custom game display name</span>
            <p className="settings-desc">
              Overrides the game name shown to friends on Steam, leave empty to show the real title.
            </p>
            <input
              type="text"
              className="settings-input"
              placeholder="Enter custom game name (e.g. Aether)"
              value={customGameName}
              onChange={(e) => setCustomGameName(e.target.value)}
            />
          </div>

          {/* Steam installation path — parte della sezione Aether (sotto il
              custom game): unica destinazione visiva, niente gruppo staccato
              tra LuaTools e Store. */}
          <div className="settings-toggle-row settings-field-column">
            <span className="settings-toggle-text">Steam Installation Path</span>
            <p className="settings-desc">The main directory path where Steam is installed on your PC, required for configuration.</p>
            <input
              type="text"
              placeholder="C:\Program Files (x86)\Steam"
              value={steamPath}
              onChange={(e) => setSteamPath(e.target.value)}
              className="settings-input"
            />
            {steamCheck.state !== 'idle' && (
              <p className={`settings-desc steam-path-status steam-path-${steamCheck.state}`}>
                {steamCheck.message}
              </p>
            )}
          </div>
          <div className="settings-toggle-row" title="Choose the Steam folder with the system file picker">
            <span className="settings-toggle-text">Browse for the Steam folder</span>
            <button
              type="button"
              className="settings-small-btn"
              style={{ minWidth: '96px', height: '33px', padding: '0 12px', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box' }}
              onClick={() => void handleBrowseSteamFolder()}
            >
              Browse
            </button>
          </div>
          <div className="settings-toggle-row" title="Auto Detect the Steam installation (registry / running Steam)">
            <span className="settings-toggle-text">Detect Steam automatically</span>
            <button
              type="button"
              className="settings-small-btn"
              style={{ minWidth: '96px', height: '33px', padding: '0 12px', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box' }}
              onClick={() => void handleDetectSteamPath()}
            >
              Auto Detect
            </button>
          </div>

          <div className="settings-toggle-row" title="Clear AetherDesk cache files such as store search, game info, Steam names and Denuvo cache. Settings and backups are preserved.">
            <span className="settings-toggle-text">Clear AetherDesk caches</span>
            <button
              type="button"
              className="settings-small-btn"
              style={{ width: '96px', minWidth: '96px', maxWidth: '96px', height: '33px', padding: '0', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box' }}
              onClick={async () => {
                try {
                  const result: string = await invoke('clear_app_caches');
                  try {
                    Object.keys(localStorage)
                      .filter((key) => key.startsWith('aether_cover_') || key.startsWith('aether_hero_'))
                      .forEach((key) => localStorage.removeItem(key));
                  } catch {}
                  showStatus(result, 'success');
                } catch (err: any) {
                  showStatus(`Failed to clear caches: ${err}`, 'error');
                }
              }}
            >
              Clear Cache
            </button>
          </div>
        </div>

        <div className="settings-separator"></div>

        {/* Hubcap API Key Section */}
        <div className="settings-group">
          <label className="settings-label">Hubcap API Key</label>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
            <p className="settings-desc">Enter your hubcapmanifest.com API key to unlock database lookups and downloads.</p>
            {hubcapUsage.hasKey && (
              <span style={{ fontSize: '12px', color: '#8f8f9e', fontWeight: 'bold', marginLeft: '12px', whiteSpace: 'nowrap' }}>
                {hubcapUsage.usage}/{hubcapUsage.limit}
              </span>
            )}
          </div>
          <div className="settings-input-wrap">
            <input
              type={showApiKey ? 'text' : 'password'}
              placeholder="Enter API key (e.g. smm_...)"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              className="settings-input"
              autoComplete="off"
              spellCheck={false}
            />
            <button
              type="button"
              className="settings-input-eye"
              title={showApiKey ? 'Hide API key' : 'Show API key'}
              aria-label={showApiKey ? 'Hide API key' : 'Show API key'}
              onClick={() => setShowApiKey((v) => !v)}
            >
              {showApiKey ? <EyeOffIcon size={16} /> : <EyeIcon size={16} />}
            </button>
          </div>
        </div>

        {/* Ryuu API Key Section */}
        <div className="settings-group">
          <label className="settings-label">Ryuu API Key</label>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
            <p className="settings-desc">Enter your generator.ryuu.lol API key to unlock downloads via Ryuu.</p>
            {ryuuKey.trim() !== '' && (
              <span style={{ fontSize: '12px', color: '#8f8f9e', fontWeight: 'bold', marginLeft: '12px', whiteSpace: 'nowrap' }}>
                50/day
              </span>
            )}
          </div>
          <div className="settings-input-wrap">
            <input
              type={showRyuuKey ? 'text' : 'password'}
              placeholder="Enter Ryuu key (e.g. V1nr...)"
              value={ryuuKey}
              onChange={(e) => setRyuuKey(e.target.value)}
              className="settings-input"
              autoComplete="off"
              spellCheck={false}
            />
            <button
              type="button"
              className="settings-input-eye"
              title={showRyuuKey ? 'Hide API key' : 'Show API key'}
              aria-label={showRyuuKey ? 'Hide API key' : 'Show API key'}
              onClick={() => setShowRyuuKey((v) => !v)}
            >
              {showRyuuKey ? <EyeOffIcon size={16} /> : <EyeIcon size={16} />}
            </button>
          </div>
        </div>

        {/* LuaTools account — OAuth tokens are stored separately from settings.json
            and protected with Windows DPAPI for the current user. */}
        <div className="settings-group">
          <label className="settings-label">LuaTools Account</label>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
            <p className="settings-desc">Log in to your lua.tools account to download from its available manifest sources.</p>
            {luaToolsAuth.signedIn && (
              <span style={{ fontSize: '12px', color: '#8f8f9e', fontWeight: 'bold', marginLeft: '12px', whiteSpace: 'nowrap' }}>
                25/day
              </span>
            )}
          </div>
          <div className="settings-toggle-row" style={{ padding: 0 }}>
            <span className="settings-toggle-text">
              {luaToolsAuth.signedIn
                ? `Connected${luaToolsAuth.displayName ? ` as ${luaToolsAuth.displayName}` : luaToolsAuth.email ? ` as ${luaToolsAuth.email}` : ''}`
                : 'Not connected'}
            </span>
            <button
              type="button"
              className="settings-small-btn"
              style={{ width: '96px', minWidth: '96px', maxWidth: '96px', height: '33px', padding: '0', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box' }}
              disabled={isLuaToolsAuthBusy && !isLuaToolsOAuthBusy}
              onClick={() => void (
                isLuaToolsOAuthBusy
                  ? cancelLuaToolsOAuth()
                  : luaToolsAuth.signedIn
                    ? handleLuaToolsSignOut()
                    : openLuaToolsLogin()
              )}
            >
              {isLuaToolsOAuthBusy ? 'Cancel' : isLuaToolsAuthBusy ? '...' : luaToolsAuth.signedIn ? 'Logout' : 'Login'}
            </button>
          </div>
        </div>


        {/* Un solo separatore tra LuaTools e Store: la sezione Steam
            Installation Path è stata spostata dentro il gruppo Aether. */}
        <div className="settings-separator"></div>

        {/* Store Section */}
        <div className="settings-group">
          <label className="settings-label">Store</label>
          <div className="settings-toggle-row" title="Show downloadable and non-downloadable add-ons (DLC) in store search results">
            <span className="settings-toggle-text">Show DLCs in the store</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={showStoreDlcs}
                onChange={(e) => setShowStoreDlcs(e.target.checked)}
              />
              <span></span>
            </label>
          </div>

          <div className="settings-toggle-row" title="Show delisted games (removed from the Steam catalog) in store search results; they are highlighted with a white border">
            <span className="settings-toggle-text">Show delisted games in the store</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={showStoreDelisted}
                onChange={(e) => setShowStoreDelisted(e.target.checked)}
              />
              <span></span>
            </label>
          </div>

          <div className="settings-toggle-row" title="Show adult-only (NSFW) games in store search results; they are highlighted with a pink border">
            <span className="settings-toggle-text">Show NSFW games in the store</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={showStoreNsfw}
                onChange={(e) => setShowStoreNsfw(e.target.checked)}
              />
              <span></span>
            </label>
          </div>

          <div className="settings-toggle-row" title="Show a Steam Store front page when no search query is active">
            <span className="settings-toggle-text">Show store front games</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={showStoreFrontGames}
                onChange={(e) => setShowStoreFrontGames(e.target.checked)}
              />
              <span></span>
            </label>
          </div>

          {showStoreFrontGames && (
            <div className="settings-toggle-row" title="Choose which Steam Store front criterion is shown by default">
              <span className="settings-toggle-text">Store front criterion</span>
              <select
                className="settings-front-filter-select"
                value={storeFrontFilter}
                onChange={(e) => setStoreFrontFilter(e.target.value)}
              >
                <option value="trending">Trending</option>
                <option value="latest">Latest</option>
                <option value="top_sellers">Top sellers</option>
                <option value="upcoming">Upcoming</option>
                <option value="discounts">Discounts</option>
              </select>
            </div>
          )}

          <div className="settings-toggle-row" title="Preferred currency for Steam prices shown in Store and Info">
            <span className="settings-toggle-text">Store price currency</span>
            <select
              className="settings-select"
              value={storeCurrency}
              onChange={(e) => setStoreCurrency(e.target.value as StoreCurrency)}
            >
              {STORE_CURRENCIES.map(({ code, label }) => (
                <option key={code} value={code}>{label}</option>
              ))}
            </select>
          </div>

          <div className="settings-toggle-row" title="After latest-version downloads, comment setManifestid pins so Steam can update the game normally. Requires a valid active Hubcap key.">
            <span className="settings-toggle-text">Download games with updates on</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={downloadGamesWithUpdatesOn}
                onChange={(e) => void handleDownloadGamesWithUpdatesChange(e.target.checked)}
              />
              <span></span>
            </label>
          </div>
        </div>

        <div className="settings-separator"></div>

        {/* Appearance — Theme, Personal Wallpaper, Alternative game cards */}
        <div className="settings-group">
          <label className="settings-label">Appearance</label>

          {/* Enable theme (first CSS in config/themes). Applies in real time:
              no "Save Settings" needed to see the theme. */}
          <div className="settings-toggle-row" title="Load the first .css file found in AetherData/config/themes after the default theme. Applies immediately.">
            <span className="settings-toggle-text">Enable custom theme</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={customCssEnabled}
                disabled={!appearanceAssets.themeExists}
                onChange={async (e) => {
                  const next = e.target.checked;
                  setCustomCssEnabled(next);
                  onCustomCssChange(next);
                  if (next) {
                    try { await invoke('ensure_custom_css'); } catch {}
                  }
                }}
              />
              <span></span>
            </label>
          </div>

          {!appearanceAssets.themeExists && (
            <p className="settings-desc settings-assets-missing">
              No theme found in AetherData/config/themes — the switch is disabled. Add a .css file to enable it.
            </p>
          )}

          {customCssEnabled && appearanceAssets.themeExists && (
            <div className="settings-appearance-sub">
              {/* Single paragraph, simple line break: the button on the right
                  is vertically centered to the whole description. */}
              <div className="settings-appearance-row">
                <p className="settings-desc">
                  The first .css file in <ClickablePath path={appearanceAssets.themesDir} onError={(msg) => showStatus(msg, 'error')} /> is applied automatically.{' '}
                  {appearanceAssets.themeName ? <>Currently active: <strong>{appearanceAssets.themeName}</strong>. </> : ''}
                  Use the button to pick a different theme.
                </p>
                {appearancePickBtn(
                  'THEME',
                  handlePickTheme,
                  !appearanceAssets.themeExists && !customCssEnabled,
                  isPicking === 'theme'
                )}
              </div>
            </div>
          )}

          {/* Enable personal wallpaper (first image in config/wallpapers) */}
          <div className="settings-toggle-row" title="Use the first image found in AetherData/config/wallpapers as the app background">
            <span className="settings-toggle-text">Enable custom wallpaper</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={personalWallpaperEnabled}
                disabled={!appearanceAssets.wallpaperExists}
                onChange={async (e) => {
                  const next = e.target.checked;
                  setPersonalWallpaperEnabled(next);
                  onPreviewPersonalWallpaper(next, personalWallpaperOpacity);
                  if (next) {
                    try { await invoke('ensure_custom_css'); } catch {}
                  }
                }}
              />
              <span></span>
            </label>
          </div>

          {!appearanceAssets.wallpaperExists && (
            <p className="settings-desc settings-assets-missing">
              No wallpaper found in AetherData/config/wallpapers — the switch is disabled. Add an image to enable it.
            </p>
          )}

          {personalWallpaperEnabled && appearanceAssets.wallpaperExists && (
            <div className="settings-appearance-sub">
              {/* Single paragraph, simple line break: the button on the right
                  is vertically centered to the whole description. */}
              <div className="settings-appearance-row">
                <p className="settings-desc">
                  The first image in <ClickablePath path={appearanceAssets.wallpapersDir} onError={(msg) => showStatus(msg, 'error')} /> is used as the app background.{' '}
                  {appearanceAssets.wallpaperName ? <>Currently active: <strong>{appearanceAssets.wallpaperName}</strong>. </> : ''}
                  Use the button to pick a different wallpaper.
                </p>
                {appearancePickBtn(
                  'WALLPAPER',
                  handlePickWallpaper,
                  !appearanceAssets.wallpaperExists && !personalWallpaperEnabled,
                  isPicking === 'wallpaper'
                )}
              </div>
              {/* Same layout as the alternative-cards fields: label left,
                  numeric input right, indented under the wallpaper switch. */}
              <div className="settings-toggle-row" title="Adjust the personal wallpaper opacity from 0 to 100">
                <span className="settings-toggle-text">Wallpaper opacity</span>
                <input
                  type="text"
                  inputMode="numeric"
                  pattern="[0-9]*"
                  className="settings-number-input"
                  value={personalWallpaperOpacity}
                  onChange={(e) => {
                    const numeric = e.target.value.replace(/\D/g, '');
                    const next = clamp0to100(Number(numeric || 0));
                    setPersonalWallpaperOpacity(next);
                    onPreviewPersonalWallpaper(personalWallpaperEnabled, next);
                  }}
                />
              </div>
            </div>
          )}

          <div className="settings-toggle-row" title="Use a custom window icon from AetherData/config/icons">
            <span className="settings-toggle-text">Enable custom icon</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={customIconEnabled}
                disabled={!appearanceAssets.iconExists}
                onChange={async (e) => {
                  const next = e.target.checked;
                  setCustomIconEnabled(next);
                  try { await invoke('ensure_custom_css'); } catch {}
                  // When turning custom icons OFF, clear the selection so the
                  // next enable starts fresh and settings never keep a stale
                  // custom path while the official icon is shown.
                  await persistAppearanceSelection(
                    next
                      ? { custom_icon_enabled: true }
                      : { custom_icon_enabled: false, icon_selected_file: '' },
                  );
                  if (!next) setIconSelectedFile('');
                  try { await invoke('apply_window_icon'); } catch (err) { console.warn('Failed to apply window icon:', err); }
                }}
              />
              <span></span>
            </label>
          </div>

          {!appearanceAssets.iconExists && (
            <p className="settings-desc settings-assets-missing">
              No icon found in AetherData/config/icons — the switch is disabled. Add an image or .ico to enable it.
            </p>
          )}

          {customIconEnabled && appearanceAssets.iconExists && (
            <div className="settings-appearance-sub">
              <div className="settings-appearance-row">
                <p className="settings-desc">
                  The first icon in <ClickablePath path={appearanceAssets.iconsDir} onError={(msg) => showStatus(msg, 'error')} /> is used as the window icon.{' '}
                  {appearanceAssets.iconName ? <>Currently active: <strong>{appearanceAssets.iconName}</strong>. </> : ''}
                  Use the button to pick a different icon.
                </p>
                {appearancePickBtn(
                  'ICON',
                  handlePickIcon,
                  !appearanceAssets.iconExists && !customIconEnabled,
                  isPicking === 'icon'
                )}
              </div>
            </div>
          )}

          {/* Alternative game cards — moved below the personal wallpaper switch */}
          <div className="settings-toggle-row" title="Use the alternate backdrop-focused game card layout in Store and Library">
            <span className="settings-toggle-text">Use alternative game cards</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={useAlternativeGameCards}
                onChange={(e) => setUseAlternativeGameCards(e.target.checked)}
              />
              <span></span>
            </label>
          </div>

          {useAlternativeGameCards && (
            <div className="settings-appearance-sub">
              <div className="settings-toggle-row" title="Adjust the backdrop image opacity of the alternative game cards from 0 to 100">
                <span className="settings-toggle-text">Backdrop opacity</span>
                <input
                  type="text"
                  inputMode="numeric"
                  pattern="[0-9]*"
                  className="settings-number-input"
                  value={alternativeCardsOpacity}
                  onChange={(e) => {
                    const numeric = e.target.value.replace(/\D/g, '');
                    const next = clamp0to100(Number(numeric || 0));
                    setAlternativeCardsOpacity(next);
                    onPreviewAlternativeCards(next, alternativeCardsFade);
                  }}
                />
              </div>
              <div className="settings-toggle-row" title="Adjust the fade-out toward the bottom of the alternative game cards from 0 (no fade) to 100 (fully dark)">
                <span className="settings-toggle-text">Backdrop fade (bottom)</span>
                <input
                  type="text"
                  inputMode="numeric"
                  pattern="[0-9]*"
                  className="settings-number-input"
                  value={alternativeCardsFade}
                  onChange={(e) => {
                    const numeric = e.target.value.replace(/\D/g, '');
                    const next = clamp0to100(Number(numeric || 0));
                    setAlternativeCardsFade(next);
                    onPreviewAlternativeCards(alternativeCardsOpacity, next);
                  }}
                />
              </div>
            </div>
          )}
        </div>

        <div className="settings-separator"></div>

        <div className="form-actions" style={{ justifyContent: 'center', gap: '12px' }}>
          <button type="submit" className="save-settings-btn" style={{ flex: '1 1 0', maxWidth: '200px' }}>
            Save Settings
          </button>
          <button
            type="button"
            className="save-settings-btn"
            style={{ flex: '1 1 0', maxWidth: '200px', backgroundColor: '#1c1c21', border: '1px solid var(--border-color)' }}
            onClick={async () => {
              // Reset to backend defaults (single source of truth, including a
              // freshly detected Steam path). The backend preserves the Library
              // filter, antivirus flag and OST ack; the UI re-renders from the
              // returned object via the same mapping as initial load.
              onPreviewPersonalWallpaper(false, 20);
              onPreviewAlternativeCards(100, 50);
              try {
                const defaults: Record<string, any> = await invoke('reset_settings_to_defaults');
                applySettingsToState(defaults);
                // Official window + shell icon after custom icon is cleared.
                try { await invoke('apply_window_icon'); } catch {}
                showStatus('Settings reset to defaults!', 'success');
                onRefreshUsage('');
                onRefreshCustomCss();
                loadAppearanceAssets();
              } catch (err: any) {
                showStatus(`Failed to reset settings: ${err}`, 'error');
              }
            }}
          >
            Reset Settings
          </button>
        </div>
      </form>

      {showHubcapUpdateWarning && (
        <HubcapUpdateWarningModal onClose={() => setShowHubcapUpdateWarning(false)} />
      )}

      {showOstWarning && (
        <OstWarningModal
          onConfirm={() => void handleOstWarningConfirm()}
          onCancel={() => setShowOstWarning(false)}
        />
      )}

      {showLuaToolsLoginModal && (
        <div className="modal-overlay" onClick={closeLuaToolsLogin}>
          <div className="modal-container" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <span className="modal-title">Login to <strong>LuaTools</strong></span>
              <button
                type="button"
                className="modal-close-btn"
                disabled={isLuaToolsAuthBusy}
                onClick={closeLuaToolsLogin}
              >
                &times;
              </button>
            </div>
            <div className="modal-separator"></div>

            <div className="modal-body">
              {luaToolsLoginMode === 'choice' ? (
                <>
                  <p className="settings-desc" style={{ margin: '0 0 8px' }}>
                    Choose how to authenticate. Both methods create the same LuaTools session for downloads.
                  </p>
                  <div style={{ display: 'grid', gap: '10px' }}>
                    <button
                      type="button"
                      className="big-action-btn luatools-login-choice"
                      onClick={() => void handleLuaToolsSignIn()}
                    >
                      <div>
                        <strong>Discord OAuth</strong>
                        <p className="settings-desc" style={{ margin: '5px 0 0' }}>
                          Easiest option. Opens Discord in your browser, but Discord/Supabase may share the email linked to your account.
                        </p>
                      </div>
                    </button>
                    <button
                      type="button"
                      className="big-action-btn luatools-login-choice"
                      onClick={() => {
                        setLuaToolsLoginMode('code');
                        setLuaToolsLoginError('');
                      }}
                    >
                      <div>
                        <strong>/login code</strong>
                        <p className="settings-desc" style={{ margin: '5px 0 0' }}>
                          More private: generate a one-time code with @Luie. LuaTools states that only your Discord username is collected, not your Discord email.
                        </p>
                      </div>
                    </button>
                  </div>
                </>
              ) : (
                <>
                  <p className="settings-desc">
                    Send <strong>/login</strong> to <strong>@Luie</strong> in Discord, then enter the 6-character code below. Codes are single-use and expire after about 5 minutes.
                  </p>
                  <input
                    className="settings-input"
                    type="text"
                    inputMode="text"
                    autoComplete="one-time-code"
                    maxLength={6}
                    placeholder="ABC123"
                    value={luaToolsLoginCode}
                    disabled={isLuaToolsAuthBusy}
                    onChange={(event) => {
                      setLuaToolsLoginCode(event.target.value.replace(/[^a-z0-9]/gi, '').toUpperCase());
                      setLuaToolsLoginError('');
                    }}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') {
                        event.preventDefault();
                        void handleLuaToolsCodeSignIn();
                      }
                    }}
                    style={{ marginTop: '14px', textTransform: 'uppercase', letterSpacing: '4px', textAlign: 'center' }}
                  />
                  {luaToolsLoginError && (
                    <div className="settings-alert error" style={{ marginTop: '10px' }}>
                      {luaToolsLoginError}
                    </div>
                  )}
                  <div className="version-actions" style={{ marginTop: '14px' }}>
                    <button
                      type="button"
                      className="panel-btn"
                      disabled={isLuaToolsAuthBusy || luaToolsLoginCode.length !== 6}
                      onClick={() => void handleLuaToolsCodeSignIn()}
                    >
                      {isLuaToolsAuthBusy ? 'Connecting...' : 'Connect'}
                    </button>
                    <button
                      type="button"
                      className="panel-btn"
                      disabled={isLuaToolsAuthBusy}
                      onClick={() => {
                        setLuaToolsLoginMode('choice');
                        setLuaToolsLoginError('');
                      }}
                    >
                      Back
                    </button>
                  </div>
                </>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

