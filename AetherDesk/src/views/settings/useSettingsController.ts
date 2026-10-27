import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { checkSteamPath, getSettings, type AppSettings } from '../../hooks/useSettings';
import type { SettingsGuard } from './types';
import type { SettingsPatch } from './settingsModel';
import { useSettingsForm } from './useSettingsForm';
import { getSteamAccounts, isValidSteamId64, parseAppIdList, type SteamAccount } from './libraryBanner';
import { isSettingsConflict, needsHubcapKey, toErrorMessage } from '../../util/errors';
import type { SetPresenceDefaultModeArgs } from '../../types/online';
import { useAppearanceSettings } from './useAppearanceSettings';
import { useLuaToolsAuth } from './useLuaToolsAuth';
import { useSteamPath } from './useSteamPath';

type StatusKind = 'info' | 'success' | 'error';

interface Params {
  /** The Settings tab is the only consumer of this bootstrap. */
  active: boolean;
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
  onRefreshCustomCss: () => Promise<void>;
  onCustomCssChange: (enabled: boolean) => void;
  onPreviewPersonalWallpaper: (enabled: boolean, opacity: number) => void;
  onPreviewAlternativeCards: (opacity: number, fade: number) => void;
  onMissingSteamPath: () => void;
  guardRef: { current: SettingsGuard | null };
}

export function useSettingsController(params: Params) {
  const { active } = params;
  const model = useSettingsForm();
  const { form, setField } = model;
  const [showApiKey, setShowApiKey] = useState(false);
  const [showRyuuKey, setShowRyuuKey] = useState(false);
  const [status, setStatus] = useState<{ text: string; type: StatusKind }>({ text: '', type: 'info' });
  const [settingsConflict, setSettingsConflict] = useState(false);
  const [settingsCorrupt, setSettingsCorrupt] = useState(false);
  const [showHubcapWarning, setShowHubcapWarning] = useState(false);
  const [resetConfirmOpen, setResetConfirmOpen] = useState(false);
  const [resetBusy, setResetBusy] = useState(false);
  const [manifestRestoreOnStartup, setManifestRestoreOnStartup] = useState(true);
  const [presenceDefaultShowOnline, setPresenceDefaultShowOnline] = useState(true);
  // Saved counterparts for switches that live outside `settings.json`: edits
  // stay pending until Save so the unsaved-changes guard covers them too.
  const manifestRestoreSavedRef = useRef(true);
  const presenceSavedRef = useRef(true);
  const [steamAccounts, setSteamAccounts] = useState<SteamAccount[]>([]);
  const [steamAccountsLoaded, setSteamAccountsLoaded] = useState(false);
  const [steamAccountsError, setSteamAccountsError] = useState(false);

  // Switches stored outside `settings.json` contribute to the same guard as
  // form fields: an edit that has not reached its command yet is unsaved.
  const extraDirty =
    presenceDefaultShowOnline !== presenceSavedRef.current ||
    manifestRestoreOnStartup !== manifestRestoreSavedRef.current;

  const showStatusSeq = useRef(0);
  const hasBootstrappedRef = useRef(false);
  const showStatus = useCallback((text: string, type: StatusKind, options?: { persist?: boolean }) => {
    const seq = (showStatusSeq.current += 1);
    setStatus({ text, type });
    if (isSettingsConflict(text)) setSettingsConflict(true);
    // Progress messages (`persist: true`) outlive the 6s expiry on purpose:
    // a 10s key validation must stay on screen the whole time instead of
    // going silent mid-operation. They are still replaced by the next
    // showStatus call, and final messages still expire after 6s.
    if (options?.persist) return;
    window.setTimeout(() => {
      if (showStatusSeq.current === seq) setStatus({ text: '', type: 'info' });
    }, 6000);
  }, []);

  // The account list is read from loginusers.vdf, which Steam rewrites when a
  // user signs in, signs out or adds an account. It is therefore refreshed on
  // mount, whenever the window regains focus, after a save and after a Steam
  // path change. The sequence guard keeps a slow, older read from overwriting a
  // newer one (and from resurrecting an error the newer read already cleared).
  const steamAccountsRequest = useRef(0);
  const reloadSteamAccounts = useCallback(async () => {
    const request = (steamAccountsRequest.current += 1);
    try {
      const accounts = await getSteamAccounts();
      if (request !== steamAccountsRequest.current) return;
      setSteamAccounts(accounts);
      setSteamAccountsError(false);
    } catch (err) {
      if (request !== steamAccountsRequest.current) return;
      setSteamAccountsError(true);
      console.warn('[settings] failed to load Steam accounts:', err);
    } finally {
      if (request === steamAccountsRequest.current) setSteamAccountsLoaded(true);
    }
  }, []);

  const applySettings = useCallback(
    (settings: AppSettings) => {
      model.applySettings(settings);
      setSettingsConflict(false);
    },
    [model]
  );
  const persistAppearance = useCallback(
    async (patch: SettingsPatch) => {
      const settings = model.getLocalSettings(patch);
      const saved = await invoke<AppSettings>('save_settings', { settings, base: model.rawSettings });
      applySettings(saved);
    },
    [applySettings, model]
  );

  const appearance = useAppearanceSettings({
    persist: persistAppearance,
    setTheme: (value) => setField('themeSelectedFile', value),
    setWallpaper: (value) => setField('wallpaperSelectedFile', value),
    setIcon: (value) => setField('iconSelectedFile', value),
    setIconEnabled: (value) => setField('customIconEnabled', value),
    refreshCustomCss: params.onRefreshCustomCss,
    showStatus,
  });
  const luaTools = useLuaToolsAuth(showStatus);
  const steamPath = useSteamPath({
    steamPath: form.steamPath,
    rawSettings: model.rawSettings,
    setSteamPath: (value) => setField('steamPath', value),
    acceptPersistedSettings: model.acceptPersistedSettings,
    refreshCustomCss: params.onRefreshCustomCss,
    // loginusers.vdf lives under the Steam install: a new path means a new
    // account list, so the picker must not keep offering the old one.
    onSteamPathSaved: () => void reloadSteamAccounts(),
    showStatus,
  });

  useEffect(() => {
    // The view stays mounted for the whole session; hydrate it the first time
    // the user opens Settings, not at startup and not on every tab return.
    if (!active || hasBootstrappedRef.current) return;
    hasBootstrappedRef.current = true;
    void invoke<AppSettings>('get_settings')
      .then(applySettings)
      .catch((err) => showStatus(`Error loading settings: ${err}`, 'error'));
    void appearance.reload();
    void invoke<boolean>('get_presence_default_mode')
      .then((value) => {
        presenceSavedRef.current = value;
        setPresenceDefaultShowOnline(value);
      })
      .catch((err) => console.warn('[settings] failed to load presence default mode:', err));
    void invoke<boolean>('get_manifest_restore_enabled')
      .then((value) => {
        manifestRestoreSavedRef.current = value;
        setManifestRestoreOnStartup(value);
      })
      .catch((err) => console.warn('[settings] failed to load manifest restore state:', err));
    void reloadSteamAccounts();
    void invoke<{ fresh: boolean; corruptFallback: boolean }>('get_settings_health')
      .then((health) => {
        if (health.corruptFallback) setSettingsCorrupt(true);
      })
      .catch((err) => console.warn('[settings] failed to read settings health:', err));
    void luaTools.refresh().catch((err) => console.warn('[settings] failed to refresh LuaTools auth:', err));
    void params.onRefreshUsage().catch((err) => console.warn('[settings] failed to refresh Hubcap usage:', err));
    // The Settings view stays mounted; bootstrap exactly once, on first activation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  // A Steam sign-in/sign-out happens in another window: refresh the picker on
  // focus instead of polling loginusers.vdf in the background.
  useEffect(() => {
    const onWindowFocus = () => void reloadSteamAccounts();
    window.addEventListener('focus', onWindowFocus);
    return () => window.removeEventListener('focus', onWindowFocus);
  }, [reloadSteamAccounts]);

  const save = useCallback(async (): Promise<boolean> => {
    if (form.libraryBannerEnabled) {
      const { ids, invalid } = parseAppIdList(form.libraryBannerAppIds);
      if (invalid.length > 0) {
        showStatus(`Invalid App ID: ${invalid.map((token) => `"${token}"`).join(', ')}`, 'error');
        return false;
      }
      if (ids.length === 0) {
        showStatus('Add at least one App ID for the library banner.', 'error');
        return false;
      }
      if (!isValidSteamId64(form.libraryBannerSteamId)) {
        showStatus('Select the owner account for the library banner.', 'error');
        return false;
      }
    }
    let invalidApiKey = false;
    let apiKeyWarning = '';
    let hubcapKeyValid = false;
    if (form.apiKey.trim()) {
      showStatus('Validating API key...', 'info', { persist: true });
      try {
        hubcapKeyValid = Boolean(await invoke('validate_hubcap_key', { apiKey: form.apiKey.trim() }));
        if (!hubcapKeyValid) {
          invalidApiKey = true;
          apiKeyWarning = 'The Hubcap API key is invalid and was cleared; the rest of your settings were saved.';
        }
      } catch (err) {
        apiKeyWarning = `Hubcap API key could not be validated (${toErrorMessage(err)}); the rest of your settings were saved anyway.`;
      }
    }
    if (form.downloadGamesWithUpdatesOn && !hubcapKeyValid) {
      setField('downloadGamesWithUpdatesOn', false);
      setShowHubcapWarning(true);
      return false;
    }
    try {
      if (!(await checkSteamPath(form.steamPath)).valid) {
        params.onMissingSteamPath();
        return false;
      }
    } catch {
      /* let the repository surface the real save error */
    }
    try {
      showStatus('Saving settings...', 'info', { persist: true });
      if (form.customCssEnabled || form.personalWallpaperEnabled) {
        try {
          await invoke('ensure_custom_css');
        } catch {
          /* best effort */
        }
      }
      const settings = model.getLocalSettings(invalidApiKey ? { hubcap_api_key: '' } : {});
      const hadDevtools = model.rawSettings.enable_webview_devtools;
      const hadIconEnabled = model.rawSettings.custom_icon_enabled;
      const saved = await invoke<AppSettings>('save_settings', { settings, base: model.rawSettings });
      applySettings(saved);
      // Switches stored outside `settings.json` only reach their command on
      // Save: that is what makes them part of the unsaved-changes guard.
      if (presenceDefaultShowOnline !== presenceSavedRef.current) {
        // IPC key `showOnline` (Rust `show_online` → lowerCamelCase), NOT the
        // TOML value, which stays "showonline": see docs/shared_contracts.md §7.
        const args: SetPresenceDefaultModeArgs = { showOnline: presenceDefaultShowOnline };
        await invoke('set_presence_default_mode', args);
        presenceSavedRef.current = presenceDefaultShowOnline;
      }
      if (manifestRestoreOnStartup !== manifestRestoreSavedRef.current) {
        await invoke('set_manifest_restore_enabled', { enabled: manifestRestoreOnStartup });
        manifestRestoreSavedRef.current = manifestRestoreOnStartup;
      }
      if (saved.custom_icon_enabled !== hadIconEnabled) {
        try {
          await invoke('apply_window_icon');
        } catch (err) {
          console.warn('[settings] failed to apply the window icon:', err);
        }
      }
      const wasCorrupt = settingsCorrupt;
      setSettingsCorrupt(false);
      if (invalidApiKey) setField('apiKey', '');
      // Devtools follow the saved setting and only change on Save (never on
      // toggle): opening right after persisting satisfies the backend gate,
      // and closing needs no gate at all.
      let devtoolsNote = '';
      if (saved.enable_webview_devtools && !hadDevtools) {
        try {
          await invoke('open_webview_devtools');
        } catch (err) {
          devtoolsNote = ` (devtools did not open: ${toErrorMessage(err)})`;
        }
      } else if (!saved.enable_webview_devtools && hadDevtools) {
        try {
          await invoke('close_webview_devtools');
        } catch {
          // Already closed or window gone: nothing to do.
        }
      }
      showStatus(
        wasCorrupt
          ? `Settings recovered and saved successfully!${devtoolsNote}`
          : apiKeyWarning || `Settings saved successfully!${devtoolsNote}`,
        apiKeyWarning || devtoolsNote ? 'error' : 'success'
      );
      await params.onRefreshUsage(invalidApiKey ? '' : form.apiKey);
      await params.onRefreshCustomCss();
      await appearance.reload();
      // The Steam path may have changed: the picker must not keep stale names.
      void reloadSteamAccounts();
      return true;
    } catch (err) {
      if (needsHubcapKey(err)) {
        setField('downloadGamesWithUpdatesOn', false);
        setShowHubcapWarning(true);
      }
      // The wrapped status text below no longer starts with the code, so the
      // conflict flag is raised here on the raw error instead of in showStatus.
      if (isSettingsConflict(err)) setSettingsConflict(true);
      showStatus(`Error during save: ${toErrorMessage(err)}`, 'error');
      return false;
    }
  }, [
    appearance,
    applySettings,
    form,
    manifestRestoreOnStartup,
    model,
    params,
    presenceDefaultShowOnline,
    reloadSteamAccounts,
    setField,
    settingsCorrupt,
    showStatus,
  ]);

  const discard = useCallback(() => {
    const baseline = model.rawSettings;
    applySettings(baseline);
    setPresenceDefaultShowOnline(presenceSavedRef.current);
    setManifestRestoreOnStartup(manifestRestoreSavedRef.current);
    params.onCustomCssChange(Boolean(baseline.custom_css_enabled));
    params.onPreviewPersonalWallpaper(
      Boolean(baseline.personal_wallpaper_enabled),
      Number(baseline.personal_wallpaper_opacity ?? 20)
    );
    params.onPreviewAlternativeCards(
      Number(baseline.alternative_cards_opacity ?? 100),
      Number(baseline.alternative_cards_fade ?? 50)
    );
  }, [applySettings, model.rawSettings, params]);

  // Installed in an effect, not during render: concurrent rendering may
  // discard a render, which would leave a stale guard behind. The cleanup
  // only clears its own installation.
  useEffect(() => {
    const guard: SettingsGuard = { isDirty: () => model.dirty || extraDirty, save, discard };
    params.guardRef.current = guard;
    return () => {
      if (params.guardRef.current === guard) params.guardRef.current = null;
    };
  }, [discard, extraDirty, model.dirty, params.guardRef, save]);

  const changeDownloadUpdates = useCallback(
    async (enabled: boolean) => {
      if (!enabled) {
        setField('downloadGamesWithUpdatesOn', false);
        return;
      }
      const key = form.apiKey.trim();
      if (!key) {
        setShowHubcapWarning(true);
        return;
      }
      try {
        if (!(await invoke<boolean>('validate_hubcap_key', { apiKey: key }))) {
          setShowHubcapWarning(true);
          return;
        }
        setField('downloadGamesWithUpdatesOn', true);
        showStatus('Hubcap key validated. Latest downloads may enable Steam updates.', 'success');
      } catch {
        setShowHubcapWarning(true);
      }
    },
    [form.apiKey, setField, showStatus]
  );

  const reloadSaved = useCallback(async () => {
    try {
      applySettings(await getSettings());
      await params.onRefreshCustomCss();
      showStatus('Saved settings reloaded. Unsaved edits discarded.', 'info');
    } catch (err) {
      showStatus(`Could not reload settings: ${String(err)}`, 'error');
    }
  }, [applySettings, params, showStatus]);

  const reset = useCallback(async () => {
    params.onPreviewPersonalWallpaper(false, 20);
    params.onPreviewAlternativeCards(100, 50);
    try {
      const defaults = await invoke<AppSettings>('reset_settings_to_defaults');
      applySettings(defaults);
      try {
        await invoke('apply_window_icon');
      } catch {
        /* best effort */
      }
      showStatus('Settings reset to defaults!', 'success');
      await params.onRefreshUsage('');
      await params.onRefreshCustomCss();
      await appearance.reload();
    } catch (err) {
      showStatus(`Failed to reset settings: ${err}`, 'error');
    }
  }, [appearance, applySettings, params, showStatus]);

  // Reset is destructive (saved settings are replaced by defaults), so the
  // Reset button only opens the confirmation; the actual action stays here.
  const requestReset = useCallback(() => setResetConfirmOpen(true), []);
  const cancelReset = useCallback(() => {
    if (!resetBusy) setResetConfirmOpen(false);
  }, [resetBusy]);
  const confirmReset = useCallback(async () => {
    setResetBusy(true);
    try {
      await reset();
    } finally {
      setResetBusy(false);
      setResetConfirmOpen(false);
    }
  }, [reset]);

  return {
    form,
    setField,
    showApiKey,
    setShowApiKey,
    showRyuuKey,
    setShowRyuuKey,
    steamCheck: steamPath.checkStatus,
    status,
    settingsConflict,
    settingsCorrupt,
    showHubcapWarning,
    setShowHubcapWarning,
    manifestRestoreOnStartup,
    setManifestRestoreOnStartup,
    presenceDefaultShowOnline,
    setPresenceDefaultShowOnline,
    steamAccounts,
    steamAccountsLoaded,
    steamAccountsError,
    appearance,
    luaTools,
    showStatus,
    save,
    reset,
    resetConfirmOpen,
    resetBusy,
    requestReset,
    cancelReset,
    confirmReset,
    browseSteam: steamPath.browse,
    detectSteam: steamPath.detect,
    changeDownloadUpdates,
    reloadSaved,
    persistAppearance,
  };
}
