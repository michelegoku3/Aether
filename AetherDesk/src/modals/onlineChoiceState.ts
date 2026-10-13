import type { AppPresenceMode, OnlineOptionKey } from '../types/online';

// Domain types live in `types/online.ts` with the other IPC contracts (same
// rule as `OnlineStatus`): this module keeps only the pure
// resolution/gating logic. Re-exported to keep existing imports working
// (`OnlineChoiceModal` takes them from here).
export type { AppPresenceMode, OnlineOptionKey } from '../types/online';

export interface OnlineChoiceContext {
  mode: AppPresenceMode;
  uco2Enabled: boolean;
  ofmePresent: boolean;
  uco2FilesPresent: boolean;
  busy: boolean;
}

export interface OnlineOptionState {
  isActive: boolean;
  isBlocked: boolean;
  tooltip: string;
}

/** Effective popup mode. An on-disk 480 spoof (OFME or UCO2) wins over
 *  default_mode=showonline: that rewrite would break invites. */
export const resolveEffectivePresenceMode = (
  aetherOnline: boolean,
  showOnlineListed: boolean,
  excluded: boolean,
  defaultShowOnline: boolean,
  spacewarSpoofPresent: boolean
): AppPresenceMode => {
  if (spacewarSpoofPresent) return 'none';
  if (aetherOnline) return 'aetheronline';
  if (showOnlineListed) return 'showonline';
  if (excluded) return 'none';
  return defaultShowOnline ? 'showonline' : 'none';
};

/**
 * ONLINE popup gating. Pure: UCO2 and Online Aether only with None; on-disk
 * OFME blocks both plus Show Online; the two Aether pipelines (Online Aether
 * / UCO2) stay mutually exclusive.
 */
export const resolveOptionState = (key: OnlineOptionKey, ctx: OnlineChoiceContext): OnlineOptionState => {
  const isActive = key === 'uco2' ? ctx.uco2Enabled : ctx.mode === key;

  if (ctx.busy) {
    return { isActive, isBlocked: true, tooltip: 'Busy…' };
  }

  if (key === 'uco2' && ctx.ofmePresent && !isActive) {
    return {
      isActive,
      isBlocked: true,
      tooltip: 'online-fix.me files are in this folder. Remove them before enabling UCO2.',
    };
  }
  if (key === 'aetheronline' && ctx.ofmePresent && !isActive) {
    return {
      isActive,
      isBlocked: true,
      tooltip:
        'online-fix.me files are in this folder. Online Aether is a different stack — keep None to use the crack.',
    };
  }
  if (key === 'showonline' && ctx.ofmePresent && !isActive) {
    return {
      isActive,
      isBlocked: true,
      tooltip: 'This folder already spoofs Spacewar (online-fix.me). Show Online would break invites — keep None.',
    };
  }
  if (key === 'aetheronline' && ctx.uco2FilesPresent && !isActive) {
    return {
      isActive,
      isBlocked: true,
      tooltip: 'UCO2 files are already in this folder. Disable or remove them first.',
    };
  }
  if (key === 'showonline' && ctx.uco2FilesPresent && !isActive) {
    return {
      isActive,
      isBlocked: true,
      tooltip: 'UCO2 files are already in this folder. Show Online remaps Spacewar and breaks invites — keep None.',
    };
  }

  if (key === 'uco2' && ctx.mode !== 'none' && !isActive) {
    return {
      isActive,
      isBlocked: false,
      tooltip: 'Switches this game to None (required for UCO2 invites) and opens setup',
    };
  }

  const pipelineConflict =
    (key === 'aetheronline' && ctx.uco2Enabled) ||
    (key === 'uco2' && ctx.mode === 'aetheronline') ||
    (key === 'showonline' && ctx.uco2Enabled);

  if (pipelineConflict && !isActive) {
    const tooltip =
      key === 'uco2'
        ? 'Disable Online Aether first (the two online stacks are exclusive)'
        : key === 'showonline'
          ? 'Disable UCO2 first — Show Online remaps Spacewar and breaks invites'
          : 'Disable UCO2 first';
    return { isActive, isBlocked: true, tooltip };
  }

  if (isActive) {
    return { isActive, isBlocked: false, tooltip: 'Currently active' };
  }
  return {
    isActive,
    isBlocked: false,
    tooltip: key === 'uco2' ? 'Open the UCO2 setup panel' : `Switch to ${key}`,
  };
};
