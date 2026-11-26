export type SchemaSource = 'steamCache' | 'aetherBackup';

export type AchievementAccountOption = {
  accountId: number;
  steamId64: string;
  personaName: string;
  isActive: boolean;
  hasStatsCache: boolean;
};

export type AchievementProgressView = {
  current: number;
  min: number;
  max: number;
};

export type AchievementItemView = {
  id: number;
  bucket: number;
  bit: number;
  apiName: string;
  name: string;
  description: string;
  hidden: boolean;
  unlocked: boolean;
  unlockTime: number;
  iconUrl?: string | null;
  iconGrayUrl?: string | null;
  progress?: AchievementProgressView | null;
};

export type AchievementStateSources = {
  steamBinPresent: boolean;
  backupBinPresent: boolean;
  backupJsonPresent: boolean;
  steamUnlockCount: number;
  backupUnlockCount: number;
  mergedFromBackup: boolean;
  pendingChanges: number;
  crc: number;
};

export type GameAchievementsSnapshot = {
  appId: number;
  gameName: string;
  schemaVersion: number;
  schemaSource: SchemaSource;
  luaManaged: boolean;
  accountId: number;
  accounts: AchievementAccountOption[];
  unlockedCount: number;
  totalCount: number;
  sources: AchievementStateSources;
  achievements: AchievementItemView[];
};

export type AchievementChange = {
  id: number;
  unlocked: boolean;
  unlockTime?: number | null;
};

export type SaveAchievementsReport = {
  appId: number;
  accountId: number;
  unlockedAfter: number;
  totalCount: number;
  appliedUnlocks: number;
  appliedRelocks: number;
  luaManaged: boolean;
};

export type GetGameAchievementsArgs = {
  appId: number;
  accountId?: number | null;
};

export type SaveGameAchievementsArgs = {
  appId: number;
  accountId: number;
  edits: AchievementChange[];
  allowSteamKill: boolean;
};
