import { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getSettings, type AppSettings } from '../hooks/useSettings';
import type { HubcapRefreshOptions, HubcapUsage, HubcapUsageStats } from './types';

const EMPTY_USAGE: HubcapUsage = { usage: 0, limit: 25, hasKey: false, active: null };

/** Loads the Hubcap quota for the active or explicitly supplied API key. */
export function useHubcapUsage() {
  const [usage, setUsage] = useState<HubcapUsage>(EMPTY_USAGE);

  const refresh = useCallback(
    async (forcedKey?: string, preloadedSettings?: AppSettings, options?: HubcapRefreshOptions) => {
      try {
        const key = forcedKey ?? (preloadedSettings ?? (await getSettings())).hubcap_api_key;
        if (!key?.trim()) {
          setUsage(EMPTY_USAGE);
          return;
        }
        const stats = await invoke<HubcapUsageStats>('get_hubcap_usage', {
          apiKey: key,
          quiet: options?.quiet ?? false,
        });
        setUsage({
          usage: stats.usage,
          limit: stats.limit,
          hasKey: true,
          active: stats.active ?? null,
          reason: stats.reason,
          keyFingerprint: stats.keyFingerprint,
          quiet: options?.quiet ?? false,
        });
      } catch (error) {
        console.error('Failed to fetch Hubcap usage:', error);
        setUsage(EMPTY_USAGE);
      }
    },
    []
  );

  return { usage, refresh };
}
