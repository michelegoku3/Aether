import { useCallback, useRef, useState } from 'react';
import type { AppSettings } from '../../hooks/useSettings';
import {
  buildSettings,
  isFormDirty,
  settingsToForm,
  type SettingsFormState,
  type SettingsPatch,
} from './settingsModel';

const EMPTY_SETTINGS = {} as AppSettings;

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

  const acceptPersistedSettings = useCallback((settings: AppSettings) => {
    baselineRef.current = settings;
    setRawSettings(settings);
  }, []);

  // Cheap pure comparison; computing directly avoids coupling memo invalidation to
  // rawSettings solely because the mutable baseline ref changed.
  const dirty = isFormDirty(form, baselineRef.current);

  return {
    form,
    setField,
    rawSettings,
    dirty,
    applySettings,
    getLocalSettings,
    acceptPersistedSettings,
  };
}
