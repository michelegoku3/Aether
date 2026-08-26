export type AppPresenceMode = 'none' | 'showonline' | 'onlinefix';
export type OnlineOptionKey = AppPresenceMode | 'uco2';

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

/** Modalità effettiva del popup. Un spoof 480 sul disco (OFME o UCO2)
 *  vince su default_mode=showonline: quel rewrite romperebbe gli inviti. */
export const resolveEffectivePresenceMode = (
  onlinefix: boolean,
  showOnlineListed: boolean,
  excluded: boolean,
  defaultShowOnline: boolean,
  spacewarSpoofPresent: boolean,
): AppPresenceMode => {
  if (spacewarSpoofPresent) return 'none';
  if (onlinefix) return 'onlinefix';
  if (showOnlineListed) return 'showonline';
  if (excluded) return 'none';
  return defaultShowOnline ? 'showonline' : 'none';
};

/**
 * Gating del popup ONLINE. Puro: UCO2 e Online Aether solo con None;
 * OFME sul disco blocca entrambi e Show Online; le due pipeline Aether
 * (Online Aether / UCO2) restano mutuamente esclusive.
 */
export const resolveOptionState = (
  key: OnlineOptionKey,
  ctx: OnlineChoiceContext,
): OnlineOptionState => {
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
  if (key === 'onlinefix' && ctx.ofmePresent && !isActive) {
    return {
      isActive,
      isBlocked: true,
      tooltip: 'online-fix.me files are in this folder. Online Aether is a different stack — keep None to use the crack.',
    };
  }
  if (key === 'showonline' && ctx.ofmePresent && !isActive) {
    return {
      isActive,
      isBlocked: true,
      tooltip: 'This folder already spoofs Spacewar (online-fix.me). Show Online would break invites — keep None.',
    };
  }
  if (key === 'onlinefix' && ctx.uco2FilesPresent && !isActive) {
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
    (key === 'onlinefix' && ctx.uco2Enabled) ||
    (key === 'uco2' && ctx.mode === 'onlinefix') ||
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
