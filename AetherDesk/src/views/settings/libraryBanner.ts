import { invoke } from '@tauri-apps/api/core';

/**
 * Cosmetic "From the library of <account>" banner helpers.
 *
 * The App ID grammar is deliberately narrow: the comma is the ONLY separator.
 * Each token is trimmed (spaces around it are ignored) and must be digits only;
 * a space inside a token makes it invalid. Empty tokens are dropped and the
 * result is unique and ascending.
 */
export interface SteamAccount {
  steamId: string;
  accountName: string;
  personaName: string;
}

export interface AppIdParseResult {
  ids: number[];
  invalid: string[];
}

const MAX_APPID = 0xffff_ffff;

export function parseAppIdList(input: string): AppIdParseResult {
  const ids: number[] = [];
  const invalid: string[] = [];
  const seen = new Set<number>();

  for (const raw of input.split(',')) {
    const token = raw.trim();
    if (!token) continue;
    if (!/^\d+$/.test(token)) {
      invalid.push(token);
      continue;
    }
    const value = Number(token);
    if (!Number.isSafeInteger(value) || value <= 0 || value > MAX_APPID) {
      invalid.push(token);
      continue;
    }
    if (!seen.has(value)) {
      seen.add(value);
      ids.push(value);
    }
  }

  ids.sort((a, b) => a - b);
  return { ids, invalid };
}

/** Renders a stored App ID list back into the single-line input format. */
export const formatAppIdList = (ids: number[] | undefined): string => (ids ?? []).join(', ');

/** Largest value an unsigned 64-bit integer (a SteamID64) can hold. */
const MAX_STEAM_ID64 = (1n << 64n) - 1n;

/**
 * True for a decimal SteamID64 that fits an unsigned 64-bit integer.
 *
 * The upper bound is not cosmetic: the setting is persisted as `u64`, so an
 * id above 2^64-1 would be rejected (and silently dropped) by the backend.
 */
export function isValidSteamId64(value: string): boolean {
  const trimmed = value.trim();
  return /^\d{1,20}$/.test(trimmed) && BigInt(trimmed) <= MAX_STEAM_ID64;
}

/** Local Steam accounts for the owner picker. Backend resolves the Steam path. */
export const getSteamAccounts = (): Promise<SteamAccount[]> => invoke<SteamAccount[]>('get_steam_accounts');
