import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emptyStatus, StatusMessage } from '../types/ui';

export interface InstalledGame {
  id: number;
  name: string;
  appId: string;
  installDir: string;
  libraryPath: string;
  gamePath: string;
  installed: boolean;
  imageUrl?: string;
  heroImageUrl?: string;
}

export const useLibraryGames = () => {
  const [games, setGames] = useState<InstalledGame[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [status, setStatus] = useState<StatusMessage>(emptyStatus());

  const loadInstalledGames = async () => {
    setIsLoading(true);
    setStatus(emptyStatus());

    try {
      const result: InstalledGame[] = await invoke('get_installed_library_games');
      setGames(result || []);
    } catch (err: any) {
      setStatus({ text: `Failed to scan Steam library: ${err}`, type: 'error' });
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadInstalledGames();
  }, []);

  return {
    games,
    isLoading,
    status,
    setStatus,
    loadInstalledGames,
  };
};
