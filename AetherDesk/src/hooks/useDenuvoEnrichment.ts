import { invoke } from '@tauri-apps/api/core';

export interface DenuvoEnrichableGame {
  appId: string;
  has_denuvo: boolean;
}

export const enrichDenuvoFlags = async <T extends DenuvoEnrichableGame>(games: T[]) => {
  const appIds = [...new Set(games.map(game => Number(game.appId)).filter(Number.isFinite))];
  if (appIds.length === 0) return games;

  const denuvoMap: Record<string, boolean> = await invoke('check_denuvo_bulk', { appIds });

  return games.map(game => ({
    ...game,
    has_denuvo: Boolean(denuvoMap[String(game.appId)]),
  }));
};
