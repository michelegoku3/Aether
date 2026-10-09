import { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getSettings, type AppSettings } from '../../hooks/useSettings';
import { useLibraryGames } from '../../hooks/useLibraryGames';
import { useModalDismiss } from '../../hooks/useModalDismiss';
import type { StoreGameResult } from '../../hooks/useStoreSearch';
import type { LuaManifestRow } from '../../modals/SpecificVersionModal';
import { emptyStatus, type StatusMessage } from '../../types/ui';

export type StoreDownloadSource = 'hubcap' | 'luatools' | 'ryuu' | 'oureveryday';

interface Params {
  onRefreshUsage?: (forcedKey?: string) => Promise<void>;
}

interface ProviderRequest {
  command: string;
  args: Record<string, unknown>;
}

const apiKeyFor = (source: StoreDownloadSource, settings: AppSettings) =>
  source === 'hubcap' ? settings.hubcap_api_key
    : source === 'ryuu' ? settings.ryuu_api_key
      : 'oureveryday_public';

export function useStoreDownload({ onRefreshUsage }: Params) {
  const { loadInstalledGames } = useLibraryGames();
  const [selectedGame, setSelectedGame] = useState<StoreGameResult | null>(null);
  const [source, setSource] = useState<StoreDownloadSource>('hubcap');
  const [status, setStatus] = useState<StatusMessage>(emptyStatus());
  const [isDownloading, setIsDownloading] = useState(false);
  const [versionGame, setVersionGame] = useState<StoreGameResult | null>(null);
  const [manifestRows, setManifestRows] = useState<LuaManifestRow[]>([]);

  useModalDismiss(() => setSelectedGame(null), isDownloading);

  const closeDownload = useCallback(() => {
    if (!isDownloading) setSelectedGame(null);
  }, [isDownloading]);

  const openDownload = useCallback(async (game: StoreGameResult) => {
    setSelectedGame(game);
    setStatus(emptyStatus());
    setIsDownloading(false);
    try {
      const [settingsResult, authResult] = await Promise.allSettled([
        getSettings(),
        invoke<{ signedIn: boolean }>('get_luatools_auth_status'),
      ]);
      if (settingsResult.status !== 'fulfilled') {
        setSource('hubcap');
        return;
      }
      const settings = settingsResult.value;
      const luaToolsSignedIn = authResult.status === 'fulfilled' && authResult.value.signedIn;
      if (settings.hubcap_api_key?.trim()) setSource('hubcap');
      else if (luaToolsSignedIn) setSource('luatools');
      else if (settings.ryuu_api_key?.trim()) setSource('ryuu');
      else setSource('hubcap');
    } catch {
      setSource('hubcap');
    }
  }, []);

  const providerRequest = async (
    game: StoreGameResult,
    mode: 'latest' | 'specific',
  ): Promise<ProviderRequest | null> => {
    setStatus({ text: 'Loading local configurations...', type: 'info' });
    const settings = await getSettings();
    const apiKey = apiKeyFor(source, settings);
    if (source === 'hubcap' && (!apiKey || apiKey.trim() === '')) {
      setStatus({ text: 'Error: Please enter your Hubcap API Key in Settings first!', type: 'error' });
      return null;
    }
    if (source === 'ryuu' && (!apiKey || apiKey.trim() === '')) {
      setStatus({ text: 'Error: Please enter your Ryuu API Key in Settings first!', type: 'error' });
      return null;
    }
    if (!settings.steam_path?.trim()) {
      setStatus({ text: 'Error: Please specify the Steam path in Settings first!', type: 'error' });
      return null;
    }

    const command = mode === 'latest'
      ? source === 'luatools' ? 'trigger_luatools_download'
        : source === 'ryuu' ? 'trigger_ryuu_download' : 'trigger_hubcap_download'
      : source === 'luatools' ? 'prepare_luatools_specific_version_download'
        : source === 'ryuu' ? 'prepare_ryuu_specific_version_download' : 'prepare_specific_version_download';
    const args = source === 'luatools'
      ? { appId: Number(game.appId), gameName: game.name || null }
      : { appId: Number(game.appId), apiKey };
    return { command, args };
  };

  const downloadLatest = async () => {
    if (!selectedGame) return;
    setIsDownloading(true);
    setStatus({ text: 'Initializing pipeline...', type: 'info' });
    try {
      const request = await providerRequest(selectedGame, 'latest');
      if (!request) { setIsDownloading(false); return; }
      setStatus({ text: `Connecting to source ${source.toUpperCase()}...`, type: 'info' });
      const result = await invoke<string>(request.command, request.args);
      loadInstalledGames();
      setStatus({ text: result, type: 'success' });
      setIsDownloading(false);
      onRefreshUsage?.();
      window.setTimeout(() => {
        setSelectedGame(null);
        setStatus(emptyStatus());
      }, 3000);
    } catch (error) {
      setStatus({ text: `Download failed: ${error}`, type: 'error' });
      setIsDownloading(false);
    }
  };

  const downloadSpecific = async () => {
    if (!selectedGame) return;
    setIsDownloading(true);
    setStatus({ text: 'Downloading Lua and preparing version table...', type: 'info' });
    try {
      const request = await providerRequest(selectedGame, 'specific');
      if (!request) { setIsDownloading(false); return; }
      const rows = await invoke<LuaManifestRow[]>(request.command, request.args);
      loadInstalledGames();
      setManifestRows((rows || []).map((row) => ({ ...row, manifestInput: '' })));
      setVersionGame(selectedGame);
      setSelectedGame(null);
      setStatus(emptyStatus());
      setIsDownloading(false);
      onRefreshUsage?.();
    } catch (error) {
      setStatus({ text: `Specific version setup failed: ${error}`, type: 'error' });
      setIsDownloading(false);
    }
  };

  const closeVersion = useCallback(() => {
    setVersionGame(null);
    setManifestRows([]);
  }, []);

  return {
    selectedGame,
    source,
    status,
    isDownloading,
    versionGame,
    manifestRows,
    setSource,
    openDownload,
    closeDownload,
    downloadLatest,
    downloadSpecific,
    closeVersion,
  };
}
