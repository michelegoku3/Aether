import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { checkSteamPath, getSettings, type AppSettings } from '../../hooks/useSettings';
import type { SettingsGuard } from './types';
import type { SettingsPatch } from './settingsModel';
import { useSettingsForm } from './useSettingsForm';
import { useAppearanceSettings } from './useAppearanceSettings';
import { useLuaToolsAuth } from './useLuaToolsAuth';
import { useSteamPath } from './useSteamPath';

type StatusKind = 'info' | 'success' | 'error';

interface Params {
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
  onRefreshCustomCss: () => Promise<void>;
  onCustomCssChange: (enabled: boolean) => void;
  onPreviewPersonalWallpaper: (enabled: boolean, opacity: number) => void;
  onPreviewAlternativeCards: (opacity: number, fade: number) => void;
  onMissingSteamPath: () => void;
  guardRef: { current: SettingsGuard | null };
}

export function useSettingsController(params: Params) {
  const model = useSettingsForm();
  const { form, setField } = model;
  const [showApiKey, setShowApiKey] = useState(false);
  const [showRyuuKey, setShowRyuuKey] = useState(false);
  const [status, setStatus] = useState<{ text: string; type: StatusKind }>({ text: '', type: 'info' });
  const [settingsConflict, setSettingsConflict] = useState(false);
  const [showHubcapWarning, setShowHubcapWarning] = useState(false);
  const [showOstWarning, setShowOstWarning] = useState(false);
  const [useOstSource, setUseOstSource] = useState(false);
  const [manifestRestoreOnStartup, setManifestRestoreOnStartup] = useState(true);
  const [presenceDefaultShowOnline, setPresenceDefaultShowOnline] = useState(true);

  const showStatus = useCallback((text: string, type: StatusKind) => {
    setStatus({ text, type });
    if (text.includes('SETTINGS_CONFLICT')) setSettingsConflict(true);
    window.setTimeout(() => setStatus({ text: '', type: 'info' }), 6000);
  }, []);

  const applySettings = useCallback((settings: AppSettings) => {
    model.applySettings(settings);
    setSettingsConflict(false);
  }, [model]);

  const persistAppearance = useCallback(async (patch: SettingsPatch) => {
    const settings = model.getLocalSettings(patch);
    const saved = await invoke<AppSettings>('save_settings', { settings, base: model.rawSettings });
    applySettings(saved);
  }, [applySettings, model]);

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
    showStatus,
  });

  useEffect(() => {
    void invoke<AppSettings>('get_settings').then(applySettings)
      .catch((err) => showStatus(`Error loading settings: ${err}`, 'error'));
    void appearance.reload();
    void invoke<boolean>('get_presence_default_mode').then(setPresenceDefaultShowOnline)
      .catch((err) => console.warn('[settings] failed to load presence default mode:', err));
    void invoke<boolean>('get_ost_source_enabled').then(setUseOstSource)
      .catch((err) => console.warn('[settings] failed to load OST source state:', err));
    void invoke<boolean>('get_manifest_restore_enabled').then(setManifestRestoreOnStartup)
      .catch((err) => console.warn('[settings] failed to load manifest restore state:', err));
    void luaTools.refresh();
    void params.onRefreshUsage();
    // The Settings view stays mounted; bootstrap exactly once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const save = useCallback(async (): Promise<boolean> => {
    let invalidApiKey = false;
    let apiKeyWarning = '';
    let hubcapKeyValid = false;
    if (form.apiKey.trim()) {
      showStatus('Validating API key...', 'info');
      try {
        hubcapKeyValid = Boolean(await invoke('validate_hubcap_key', { apiKey: form.apiKey.trim() }));
        if (!hubcapKeyValid) {
          invalidApiKey = true;
          apiKeyWarning = 'The Hubcap API key is invalid and was cleared; the rest of your settings were saved.';
        }
      } catch (err) {
        apiKeyWarning = `Hubcap API key could not be validated (${err}); the rest of your settings were saved anyway.`;
      }
    }
    if (form.downloadGamesWithUpdatesOn && !hubcapKeyValid) {
      setField('downloadGamesWithUpdatesOn', false); setShowHubcapWarning(true); return false;
    }
    try {
      if (!(await checkSteamPath(form.steamPath)).valid) { params.onMissingSteamPath(); return false; }
    } catch { /* let the repository surface the real save error */ }
    try {
      showStatus('Saving settings...', 'info');
      if (form.customCssEnabled || form.personalWallpaperEnabled) {
        try { await invoke('ensure_custom_css'); } catch { /* best effort */ }
      }
      const settings = model.getLocalSettings(invalidApiKey ? { hubcap_api_key: '' } : {});
      const saved = await invoke<AppSettings>('save_settings', { settings, base: model.rawSettings });
      applySettings(saved);
      if (invalidApiKey) setField('apiKey', '');
      showStatus(apiKeyWarning || 'Settings saved successfully!', apiKeyWarning ? 'error' : 'success');
      await params.onRefreshUsage(invalidApiKey ? '' : form.apiKey);
      await params.onRefreshCustomCss();
      await appearance.reload();
      return true;
    } catch (err) {
      const message = String(err);
      if (message.includes('HUBCAP_KEY_REQUIRED_FOR_UPDATES')) {
        setField('downloadGamesWithUpdatesOn', false); setShowHubcapWarning(true);
      }
      showStatus(`Error during save: ${message}`, 'error'); return false;
    }
  }, [appearance, applySettings, form, model, params, setField, showStatus]);

  const discard = useCallback(() => {
    const baseline = model.rawSettings;
    applySettings(baseline);
    params.onCustomCssChange(Boolean(baseline.custom_css_enabled));
    params.onPreviewPersonalWallpaper(Boolean(baseline.personal_wallpaper_enabled), Number(baseline.personal_wallpaper_opacity ?? 20));
    params.onPreviewAlternativeCards(Number(baseline.alternative_cards_opacity ?? 100), Number(baseline.alternative_cards_fade ?? 50));
  }, [applySettings, model.rawSettings, params]);

  params.guardRef.current = { isDirty: () => model.dirty, save, discard };
  useEffect(() => () => { params.guardRef.current = null; }, [params.guardRef]);

  const changeDownloadUpdates = useCallback(async (enabled: boolean) => {
    if (!enabled) { setField('downloadGamesWithUpdatesOn', false); return; }
    const key = form.apiKey.trim();
    if (!key) { setShowHubcapWarning(true); return; }
    try {
      if (!await invoke<boolean>('validate_hubcap_key', { apiKey: key })) { setShowHubcapWarning(true); return; }
      setField('downloadGamesWithUpdatesOn', true);
      showStatus('Hubcap key validated. Latest downloads may enable Steam updates.', 'success');
    } catch { setShowHubcapWarning(true); }
  }, [form.apiKey, setField, showStatus]);

  const confirmOst = useCallback(async () => {
    try {
      try { await invoke('acknowledge_ost_warning'); } catch { /* best effort */ }
      await invoke('set_ost_source_enabled', { enabled: true });
      setField('ostWarningAcknowledged', true); setUseOstSource(true); setShowOstWarning(false);
      model.acceptPersistedSettings({ ...model.rawSettings, ost_warning_acknowledged: true });
    } catch (err) { setShowOstWarning(false); showStatus(`Failed to set OST pattern source: ${err}`, 'error'); }
  }, [model, setField, showStatus]);

  const reloadSaved = useCallback(async () => {
    try { applySettings(await getSettings()); await params.onRefreshCustomCss(); showStatus('Saved settings reloaded. Unsaved edits discarded.', 'info'); }
    catch (err) { showStatus(`Could not reload settings: ${String(err)}`, 'error'); }
  }, [applySettings, params, showStatus]);

  const reset = useCallback(async () => {
    params.onPreviewPersonalWallpaper(false, 20); params.onPreviewAlternativeCards(100, 50);
    try {
      const defaults = await invoke<AppSettings>('reset_settings_to_defaults'); applySettings(defaults);
      try { await invoke('apply_window_icon'); } catch { /* best effort */ }
      showStatus('Settings reset to defaults!', 'success');
      await params.onRefreshUsage(''); await params.onRefreshCustomCss(); await appearance.reload();
    } catch (err) { showStatus(`Failed to reset settings: ${err}`, 'error'); }
  }, [appearance, applySettings, params, showStatus]);

  return {
    form, setField, showApiKey, setShowApiKey, showRyuuKey, setShowRyuuKey,
    steamCheck: steamPath.checkStatus,
    status, settingsConflict, showHubcapWarning, setShowHubcapWarning, showOstWarning, setShowOstWarning,
    useOstSource, setUseOstSource, manifestRestoreOnStartup, setManifestRestoreOnStartup,
    presenceDefaultShowOnline, setPresenceDefaultShowOnline, appearance, luaTools,
    showStatus, save, reset, browseSteam: steamPath.browse, detectSteam: steamPath.detect,
    changeDownloadUpdates, confirmOst, reloadSaved,
    persistAppearance,
  };
}
