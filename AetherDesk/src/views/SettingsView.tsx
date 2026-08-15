import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface SettingsViewProps {
  hubcapUsage: { usage: number; limit: number; hasKey: boolean };
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
  onRefreshCustomCss: () => Promise<void>;
  onCustomCssChange: (enabled: boolean) => void;
  onPreviewPersonalWallpaper: (enabled: boolean, opacity: number) => void;
  onPreviewAlternativeCards: (opacity: number, fade: number) => void;
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

const clamp0to100 = (value: number) => Math.max(0, Math.min(100, Number.isFinite(value) ? value : 0));

export const SettingsView = ({ hubcapUsage, onRefreshUsage, onRefreshCustomCss, onCustomCssChange, onPreviewPersonalWallpaper, onPreviewAlternativeCards }: SettingsViewProps) => {
  const [apiKey, setApiKey] = useState('');
  const [steamPath, setSteamPath] = useState('C:\\Program Files (x86)\\Steam');
  const [activeLibrary, setActiveLibrary] = useState('');
  const [showStoreDlcs, setShowStoreDlcs] = useState(false);
  const [showStoreNsfw, setShowStoreNsfw] = useState(true);
  const [showStoreDelisted, setShowStoreDelisted] = useState(true);
  const [downloadGamesWithUpdatesOn, setDownloadGamesWithUpdatesOn] = useState(true);
  const [showStoreFrontGames, setShowStoreFrontGames] = useState(true);
  const [useAlternativeGameCards, setUseAlternativeGameCards] = useState(false);
  const [enableWebviewDevtools, setEnableWebviewDevtools] = useState(false);
  const [enableTestUpdates, setEnableTestUpdates] = useState(false);
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
  const [storeCurrency, setStoreCurrency] = useState<'eur' | 'usd' | 'jpy'>('eur');

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

  // Load settings from the backend when the component mounts
  useEffect(() => {
    const loadSettings = async () => {
      try {
        const settings: any = await invoke('get_settings');
        if (settings) {
          setRawSettings(settings);
          setApiKey(settings.hubcap_api_key || '');
          setSteamPath(settings.steam_path || 'C:\\Program Files (x86)\\Steam');
          setActiveLibrary(settings.active_library || '');
          setShowStoreDlcs(Boolean(settings.show_store_dlcs));
          // These two default to enabled: only an explicit `false` turns them off.
          setShowStoreNsfw(settings.show_store_nsfw !== false);
          setShowStoreDelisted(settings.show_store_delisted !== false);
          setDownloadGamesWithUpdatesOn(settings.download_games_with_updates_on !== false);
          setShowStoreFrontGames(settings.show_store_front_games !== false);
          setUseAlternativeGameCards(Boolean(settings.use_alternative_game_cards));
          setEnableWebviewDevtools(Boolean(settings.enable_webview_devtools));
          setEnableTestUpdates(Boolean(settings.enable_test_updates));
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
          setStoreCurrency(['usd', 'jpy'].includes(settings.store_currency) ? settings.store_currency : 'eur');
        }
      } catch (err: any) {
        showStatus(`Error loading settings: ${err}`, 'error');
      }
    };
    loadSettings();
    loadAppearanceAssets();
    onRefreshUsage();
  }, []);

  /** Constructs the full current settings object from React state variables,
   *  merging over `rawSettings`. Using this helper ensures that any save
   *  operation (manual Save, theme selection, wallpaper selection) always
   *  preserves every modified setting in React memory, never overwriting
   *  `hubcap_api_key` or other fields with stale `rawSettings`. */
  const buildCurrentSettings = (overrides: Record<string, any> = {}) => ({
    ...rawSettings,
    hubcap_api_key: apiKey,
    steam_path: steamPath,
    active_library: activeLibrary,
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
    store_front_filter: storeFrontFilter,
    store_currency: storeCurrency,
    ...overrides,
  });

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();

    // Validate API key when present
    if (apiKey.trim()) {
      showStatus('Validating API key...', 'info');
      try {
        const isValid: any = await invoke('validate_hubcap_key', { apiKey });
        if (!isValid) {
          showStatus('Invalid API key. Settings not saved.', 'error');
          return;
        }
      } catch (err: any) {
        showStatus(`API key validation failed: ${err}`, 'error');
        return;
      }
    }

    // Persist settings
    try {
      showStatus('Saving settings...', 'info');
      // If the user just enabled Custom CSS, ensure the file exists before
      // saving so the next editor open does not find an empty folder.
      if (customCssEnabled || personalWallpaperEnabled) {
        try { await invoke('ensure_custom_css'); } catch {}
      }
      const newSettings = buildCurrentSettings();
      await invoke('save_settings', { settings: newSettings });
      setRawSettings(newSettings);
      showStatus('Settings saved successfully!', 'success');
      onRefreshUsage(apiKey);
      onRefreshCustomCss();
      loadAppearanceAssets();
    } catch (err: any) {
      showStatus(`Error during save: ${err}`, 'error');
    }
  };

  /** Persists a freshly picked theme/wallpaper file immediately (the selection
   *  is part of settings) and refreshes the live preview. Using buildCurrentSettings
   *  and updating rawSettings guarantees we never revert toggles or wipe out
   *  an un-saved API key. */
  const persistAppearanceSelection = async (patch: Record<string, any>) => {
    const newSettings = buildCurrentSettings(patch);
    await invoke('save_settings', { settings: newSettings });
    setRawSettings(newSettings);
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
          <label className="settings-label">AETHER</label>
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
          <input 
            type="password" 
            placeholder="Enter API key (e.g. smm_...)"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            className="settings-input"
          />
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
          <input 
            type="password" 
            placeholder="Enter Ryuu key (e.g. V1nr...)"
            value={ryuuKey}
            onChange={(e) => setRyuuKey(e.target.value)}
            className="settings-input"
          />
        </div>

        <div className="settings-separator"></div>

        {/* Steam Path Section */}
        <div className="settings-group">
          <label className="settings-label">Steam Installation Path</label>
          <p className="settings-desc">The main directory path where Steam is installed on your PC, required for configuration.</p>
          <input 
            type="text" 
            placeholder="C:\Program Files (x86)\Steam"
            value={steamPath}
            onChange={(e) => setSteamPath(e.target.value)}
            className="settings-input"
          />
        </div>

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
              onChange={(e) => setStoreCurrency(e.target.value as 'eur' | 'usd' | 'jpy')}
            >
              <option value="eur">Euro (€)</option>
              <option value="usd">Dollar ($)</option>
              <option value="jpy">Yen (¥)</option>
            </select>
          </div>

          <div className="settings-toggle-row" title="After latest-version downloads, comment setManifestid pins so Steam can update the game normally">
            <span className="settings-toggle-text">Download games with updates on</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={downloadGamesWithUpdatesOn}
                onChange={(e) => setDownloadGamesWithUpdatesOn(e.target.checked)}
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
                  The first .css file in <code className="settings-path" title={appearanceAssets.themesDir}>{appearanceAssets.themesDir}</code> is applied automatically.{' '}
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
          <div className="settings-toggle-row" title="Use the first image found in AetherData/config/wallpapers as the app background.">
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
                  The first image in <code className="settings-path" title={appearanceAssets.wallpapersDir}>{appearanceAssets.wallpapersDir}</code> is used as the app background.{' '}
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
              <div className="settings-toggle-row" title="Adjust the personal wallpaper opacity from 0 to 100.">
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

          <div className="settings-toggle-row" title="Use a custom window icon from AetherData/config/icons.">
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
                  The first icon in <code className="settings-path" title={appearanceAssets.iconsDir}>{appearanceAssets.iconsDir}</code> is used as the window icon.{' '}
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
          <div className="settings-toggle-row" title="Use the alternate backdrop-focused game card layout in Store and Library.">
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
              <div className="settings-toggle-row" title="Adjust the backdrop image opacity of the alternative game cards from 0 to 100.">
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
              <div className="settings-toggle-row" title="Adjust the fade-out toward the bottom of the alternative game cards from 0 (no fade) to 100 (fully dark).">
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
              // Reset to defaults (matching Rust AppSettings::default)
              setApiKey('');
              setRyuuKey('');
              setSteamPath('C:\\Program Files (x86)\\Steam');
              setActiveLibrary('');
              setShowStoreDlcs(false);
              setShowStoreNsfw(true);
              setShowStoreDelisted(true);
              setDownloadGamesWithUpdatesOn(true);
              setShowStoreFrontGames(true);
              setUseAlternativeGameCards(false);
              setEnableWebviewDevtools(false);
              setEnableTestUpdates(false);
              setStoreFrontFilter('upcoming');
              setCustomCssEnabled(false);
              setPersonalWallpaperEnabled(false);
              setPersonalWallpaperOpacity(20);
              setAlternativeCardsOpacity(100);
              setAlternativeCardsFade(50);
              setThemeSelectedFile('');
              setWallpaperSelectedFile('');
              setCustomIconEnabled(false);
              setIconSelectedFile('');
              onPreviewPersonalWallpaper(false, 20);
              onPreviewAlternativeCards(100, 50);
              setStoreCurrency('eur');
              try {
                await invoke('save_settings', {
                  settings: {
                    ...rawSettings,
                    hubcap_api_key: '',
                    ryuu_api_key: '',
                    steam_path: 'C:\\Program Files (x86)\\Steam',
                    active_library: '',
                    show_store_dlcs: false,
                    show_store_nsfw: true,
                    show_store_delisted: true,
                    download_games_with_updates_on: true,
                    show_store_front_games: true,
                    use_alternative_game_cards: false,
                    enable_webview_devtools: false,
                    enable_test_updates: false,
                    store_front_filter: 'upcoming',
                    custom_css_enabled: false,
                    personal_wallpaper_enabled: false,
                    personal_wallpaper_opacity: 20,
                    wallpaper_selected_file: '',
                    theme_selected_file: '',
                    custom_icon_enabled: false,
                    icon_selected_file: '',
                    alternative_cards_opacity: 100,
                    alternative_cards_fade: 50,
                    store_currency: 'eur',
                    // Library toolbar filter is owned by LibraryView; keep the
                    // user's current choice across a Settings reset.
                    library_install_filter: rawSettings.library_install_filter || 'all',
                    antivirus_exclusion_done: rawSettings.antivirus_exclusion_done ?? false
                  }
                });
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
    </div>
  );
};
