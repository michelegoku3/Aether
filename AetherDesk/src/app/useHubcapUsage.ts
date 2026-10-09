import { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getSettings, type AppSettings } from '../hooks/useSettings';
import type { HubcapUsage, HubcapUsageStats } from './types';

const EMPTY_USAGE: HubcapUsage = { usage: 0, limit: 1500, hasKey: false };

/** Loads the Hubcap quota for the active or explicitly supplied API key. */
export function useHubcapUsage() {
  const [usage, setUsage] = useState<HubcapUsage>(EMPTY_USAGE);

  const refresh = useCallback(async (forcedKey?: string, preloadedSettings?: AppSettings) => {
    try {
      const key = forcedKey ?? (preloadedSettings ?? await getSettings()).hubcap_api_key;
      if (!key?.trim()) {
        setUsage(EMPTY_USAGE);
        return;
      }
      const stats = await invoke<HubcapUsageStats>('get_hubcap_usage', { apiKey: key });
      setUsage({ usage: stats.usage, limit: stats.limit, hasKey: true });
    } catch (error) {
      console.error('Failed to fetch Hubcap usage:', error);
      setUsage(EMPTY_USAGE);
    }
  }, []);

  return { usage, refresh };
}
