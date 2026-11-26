import { invoke } from '@tauri-apps/api/core';

export interface PcDrmEnrichableGame {
  appId: string;
  name: string;
  has_denuvo?: boolean;
  drm_protection?: string;
}

export const enrichPcDrm = async <T extends PcDrmEnrichableGame>(games: T[]) => {
  const validGames = games.filter((game) => Number.isFinite(Number(game.appId)));
  const appIds = validGames.map((game) => Number(game.appId));
  const names = validGames.map((game) => game.name);
  if (appIds.length === 0) return games;

  const drmMap: Record<string, string> = await invoke('check_pc_drm_bulk', { appIds, names });

  return games.map((game) => {
    const incoming = (drmMap[String(game.appId)] ?? '').trim();
    if (!incoming) return game;

    const existing = (game.drm_protection ?? '').trim();
    const incomingIsDenuvo =
      incoming.toLowerCase().includes('denuvo') ||
      incoming.toLowerCase().includes('enuvo') ||
      incoming.toLowerCase().includes('nuvo');
    const existingIsDenuvo =
      existing.toLowerCase().includes('denuvo') ||
      existing.toLowerCase().includes('enuvo') ||
      existing.toLowerCase().includes('nuvo');
    const incomingIsNone = incoming.toLowerCase() === 'none';

    let drm = game.drm_protection;
    if (incomingIsDenuvo) {
      drm = 'Denuvo';
    } else if (incomingIsNone && existingIsDenuvo) {
      drm = existing;
    } else if (!existingIsDenuvo) {
      drm = incoming;
    }

    return {
      ...game,
      drm_protection: drm,
      has_denuvo: game.has_denuvo || incomingIsDenuvo,
    };
  });
};
