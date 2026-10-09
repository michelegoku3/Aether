import { useCallback, useMemo, useRef, useState } from 'react';
import { getSettings, type AppSettings } from '../../hooks/useSettings';
import {
  buildSettings,
  isFormDirty,
  settingsToForm,
  type SettingsFormState,
  type SettingsPatch,
} from './settingsModel';

const EMPTY_SETTINGS = {} as AppSettings;

export interface PersistResult {
  merged: AppSettings;
  conflictedKeys: string[];
}

export function useSettingsForm() {
  const [rawSettings, setRawSettings] = useState<AppSettings>(EMPTY_SETTINGS);
  const [form, setForm] = useState<SettingsFormState>(() => settingsToForm(EMPTY_SETTINGS));
  const baselineRef = useRef<AppSettings>(EMPTY_SETTINGS);

  const setField = useCallback(<K extends keyof SettingsFormState>(
    key: K,
    value: SettingsFormState[K],
  ) => {
    setForm((current) => ({ ...current, [key]: value }));
  }, []);

  const applySettings = useCallback((settings: AppSettings) => {
    baselineRef.current = settings;
    setRawSettings(settings);
    setForm(settingsToForm(settings));
  }, []);

  const getLocalSettings = useCallback(
    (overrides: SettingsPatch = {}) => buildSettings(form, rawSettings, overrides),
    [form, rawSettings],
  );

  const mergeWithFreshSettings = useCallback(async (
    local: AppSettings,
    baseline = baselineRef.current,
  ): Promise<PersistResult> => {
    const fresh = await getSettings();
    const changedKeys = new Set<string>();
    const allKeys = new Set([...Object.keys(baseline), ...Object.keys(local)]);
    allKeys.forEach((key) => {
      if (JSON.stringify(local[key]) !== JSON.stringify(baseline[key])) changedKeys.add(key);
    });

    const conflictedKeys = [...changedKeys].filter(
      (key) => JSON.stringify(fresh[key]) !== JSON.stringify(baseline[key]),
    );
    const patch = Object.fromEntries([...changedKeys].map((key) => [key, local[key]]));
    return { merged: { ...fresh, ...patch }, conflictedKeys };
  }, []);

  const acceptPersistedSettings = useCallback((settings: AppSettings) => {
    baselineRef.current = settings;
    setRawSettings(settings);
  }, []);

  const dirty = useMemo(() => isFormDirty(form, baselineRef.current), [form, rawSettings]);

  return {
    form,
    setForm,
    setField,
    rawSettings,
    baselineRef,
    dirty,
    applySettings,
    getLocalSettings,
    mergeWithFreshSettings,
    acceptPersistedSettings,
  };
}
