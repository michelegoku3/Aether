import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalShell } from '../ui/ModalShell';
import { TrophyIcon } from '../ui/icons';
import { SteamCloseConfirmModal } from './SteamCloseConfirmModal';
import { toErrorMessage } from '../util/errors';
import type {
  AchievementChange,
  AchievementItemView,
  GameAchievementsSnapshot,
  GetGameAchievementsArgs,
  SaveAchievementsReport,
  SaveGameAchievementsArgs,
} from '../types/achievements';

export interface AchievementsModalProps {
  appId: number;
  gameName: string;
  onClose: () => void;
  onStatus?: (message: string, type: 'info' | 'success' | 'error') => void;
}

type BulkActionState = 'unlockAll' | 'lockAll' | null;

const STEAM_APP_IMAGE_HOSTS = [
  'cdn.fastly.steamstatic.com',
  'cdn.akamai.steamstatic.com',
  'steamcdn-a.akamaihd.net',
  'media.steampowered.com',
];

const buildIconCandidates = (primary?: string | null): string[] => {
  const trimmed = primary?.trim();
  if (!trimmed) return [];
  const marker = '/steamcommunity/public/images/apps/';
  const markerIdx = trimmed.indexOf(marker);
  if (markerIdx === -1) {
    return [trimmed];
  }
  const suffix = trimmed.slice(markerIdx);
  const candidates: string[] = [trimmed];
  for (const host of STEAM_APP_IMAGE_HOSTS) {
    const url = `https://${host}${suffix}`;
    if (!candidates.includes(url)) {
      candidates.push(url);
    }
  }
  return candidates;
};

const formatUnlockTime = (unixSeconds: number): string => {
  if (!unixSeconds || unixSeconds <= 0) return 'Unlocked (date unknown)';
  try {
    return `Unlocked ${new Date(unixSeconds * 1000).toLocaleString()}`;
  } catch {
    return `Unlocked (${unixSeconds})`;
  }
};

const stripErrorCode = (raw: string): { code: string | null; text: string } => {
  for (const prefix of ['NO_SCHEMA:', 'NO_ACCOUNT:', 'STEAM_RUNNING:']) {
    if (raw.startsWith(prefix)) {
      return {
        code: prefix.slice(0, -1),
        text: raw.slice(prefix.length).trim(),
      };
    }
  }
  return { code: null, text: raw };
};

export const AchievementsModal = ({ appId, gameName, onClose, onStatus }: AchievementsModalProps) => {
  const [snapshot, setSnapshot] = useState<GameAchievementsSnapshot | null>(null);
  const [selectedAccountId, setSelectedAccountId] = useState<number | undefined>(undefined);
  const [pending, setPending] = useState<Record<number, boolean>>({});
  const [bulkAction, setBulkAction] = useState<BulkActionState>(null);
  const [search, setSearch] = useState('');
  const [revealHidden, setRevealHidden] = useState(false);
  const [iconAttempts, setIconAttempts] = useState<Record<string, number>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmSteamClose, setConfirmSteamClose] = useState(false);

  const load = useCallback(
    async (accountId?: number) => {
      setLoading(true);
      setError(null);
      try {
        const loadArgs: GetGameAchievementsArgs = {
          appId,
          accountId: accountId ?? null,
        };
        const data = await invoke<GameAchievementsSnapshot>('get_game_achievements', loadArgs);
        setSnapshot(data);
        setSelectedAccountId(data.accountId);
        setPending({});
        setBulkAction(null);
      } catch (err: unknown) {
        setSnapshot(null);
        setError(toErrorMessage(err));
      } finally {
        setLoading(false);
      }
    },
    [appId]
  );

  useEffect(() => {
    void load();
  }, [load]);

  const edits: AchievementChange[] = useMemo(
    () =>
      Object.entries(pending).map(([id, unlocked]) => ({
        id: Number(id),
        unlocked,
      })),
    [pending]
  );

  const effectiveUnlockedCount = useMemo(() => {
    if (!snapshot) return 0;
    return snapshot.achievements.reduce((acc, item) => {
      const state = pending[item.id] ?? item.unlocked;
      return acc + (state ? 1 : 0);
    }, 0);
  }, [snapshot, pending]);

  const totalCount = snapshot?.totalCount ?? 0;
  const percent = totalCount > 0 ? Math.round((effectiveUnlockedCount / totalCount) * 100) : 0;

  const toggleItem = (item: AchievementItemView) => {
    const currentEffective = pending[item.id] ?? item.unlocked;
    const nextEffective = !currentEffective;
    setPending((prev) => {
      const next = { ...prev };
      if (nextEffective === item.unlocked) {
        delete next[item.id];
      } else {
        next[item.id] = nextEffective;
      }
      if (Object.keys(next).length === 0) {
        setBulkAction(null);
      }
      return next;
    });
  };

  const handleUnlockAllClick = () => {
    if (bulkAction === 'unlockAll') {
      setPending({});
      setBulkAction(null);
      return;
    }
    if (!snapshot) return;
    const next: Record<number, boolean> = {};
    for (const item of snapshot.achievements) {
      if (!item.unlocked) {
        next[item.id] = true;
      }
    }
    setPending(next);
    setBulkAction(Object.keys(next).length > 0 ? 'unlockAll' : null);
  };

  const handleLockAllClick = () => {
    if (bulkAction === 'lockAll') {
      setPending({});
      setBulkAction(null);
      return;
    }
    if (!snapshot) return;
    const next: Record<number, boolean> = {};
    for (const item of snapshot.achievements) {
      if (item.unlocked) {
        next[item.id] = false;
      }
    }
    setPending(next);
    setBulkAction(Object.keys(next).length > 0 ? 'lockAll' : null);
  };

  const matchesSearch = useCallback(
    (item: AchievementItemView) => {
      const q = search.trim().toLowerCase();
      if (!q) return true;
      const isMasked = item.hidden && !item.unlocked && !revealHidden;
      if (isMasked) {
        return 'hidden achievement'.includes(q);
      }
      return (
        item.name.toLowerCase().includes(q) ||
        item.description.toLowerCase().includes(q) ||
        item.apiName.toLowerCase().includes(q)
      );
    },
    [search, revealHidden]
  );

  // Partition strictly by saved on-disk state (`item.unlocked`) so toggling a
  // checkbox never teleports the row between sections mid-edit.
  const { unlockedItems, lockedItems } = useMemo(() => {
    const unlockedList: AchievementItemView[] = [];
    const lockedList: AchievementItemView[] = [];
    if (!snapshot) return { unlockedItems: unlockedList, lockedItems: lockedList };

    for (const item of snapshot.achievements) {
      if (!matchesSearch(item)) continue;
      if (item.unlocked) {
        unlockedList.push(item);
      } else {
        lockedList.push(item);
      }
    }
    return { unlockedItems: unlockedList, lockedItems: lockedList };
  }, [snapshot, matchesSearch]);

  const commitSave = async (allowSteamKill: boolean) => {
    if (!snapshot || edits.length === 0) return;
    setSaving(true);
    setError(null);
    try {
      const saveArgs: SaveGameAchievementsArgs = {
        appId,
        accountId: snapshot.accountId,
        edits,
        allowSteamKill,
      };
      const report = await invoke<SaveAchievementsReport>('save_game_achievements', saveArgs);
      onStatus?.(
        `Saved achievements for ${snapshot.gameName}: ${report.unlockedAfter}/${report.totalCount} unlocked.`,
        'success'
      );
      await load(snapshot.accountId);
    } catch (err: unknown) {
      const msg = toErrorMessage(err);
      if (!allowSteamKill && msg.startsWith('STEAM_RUNNING:')) {
        setConfirmSteamClose(true);
      } else {
        setError(msg);
      }
    } finally {
      setSaving(false);
    }
  };

  const renderAchievementRow = (item: AchievementItemView) => {
    const effectiveUnlocked = pending[item.id] ?? item.unlocked;
    const isModified = pending[item.id] !== undefined && pending[item.id] !== item.unlocked;
    const isMasked = item.hidden && !item.unlocked && !revealHidden;

    const baseIconUrl = effectiveUnlocked ? item.iconUrl : (item.iconGrayUrl ?? item.iconUrl);
    const candidates = buildIconCandidates(baseIconUrl);
    const attemptKey = `${item.id}:${baseIconUrl ?? ''}`;
    const attemptIdx = iconAttempts[attemptKey] ?? 0;
    const activeIconUrl = attemptIdx < candidates.length ? candidates[attemptIdx] : null;

    const displayName = isMasked ? 'Hidden achievement' : item.name;
    const displayDesc = isMasked
      ? 'Details for this hidden achievement stay veiled until unlocked or revealed.'
      : item.description || item.apiName;

    const progressPct =
      item.progress && item.progress.max > item.progress.min
        ? Math.max(
            0,
            Math.min(
              100,
              Math.round(((item.progress.current - item.progress.min) / (item.progress.max - item.progress.min)) * 100)
            )
          )
        : 0;

    return (
      <label
        key={item.id}
        className={`ach-row${effectiveUnlocked ? ' ach-row--unlocked' : ' ach-row--locked'}${isModified ? ' ach-row--dirty' : ''}`}
      >
        <div className={`ach-icon-box${!effectiveUnlocked && !item.iconGrayUrl ? ' ach-icon-box--dim' : ''}`}>
          {activeIconUrl ? (
            <img
              src={activeIconUrl}
              alt={displayName}
              className="ach-icon-img"
              referrerPolicy="no-referrer"
              onError={() =>
                setIconAttempts((prev) => ({
                  ...prev,
                  [attemptKey]: (prev[attemptKey] ?? 0) + 1,
                }))
              }
            />
          ) : (
            <TrophyIcon size={24} />
          )}
        </div>

        <div className="ach-meta">
          <div className="ach-title-line">
            <span className="ach-name">{displayName}</span>
            {isModified && (
              <span className={`ach-badge ${effectiveUnlocked ? 'ach-badge--unlock' : 'ach-badge--relock'}`}>
                {effectiveUnlocked ? 'Will unlock' : 'Will relock'}
              </span>
            )}
          </div>
          <p className="ach-desc">{displayDesc}</p>
          {item.progress && !isMasked && (
            <div className="ach-counter">
              <div className="ach-counter-track">
                <div className="ach-counter-fill" style={{ width: `${progressPct}%` }} />
              </div>
              <span className="ach-counter-label">
                {item.progress.current} / {item.progress.max}
              </span>
            </div>
          )}
          <span className="ach-time">
            {effectiveUnlocked
              ? isModified
                ? 'Will record unlock on save'
                : formatUnlockTime(item.unlockTime)
              : 'Locked'}
          </span>
        </div>

        <div className="ach-toggle">
          <input
            type="checkbox"
            checked={effectiveUnlocked}
            disabled={saving}
            aria-label={displayName}
            onChange={() => toggleItem(item)}
          />
        </div>
      </label>
    );
  };

  const parsedError = error ? stripErrorCode(error) : null;
  const displayTitle = snapshot?.gameName ?? gameName;
  const headerTitle = (
    <>
      Achievements: <strong style={{ color: '#ffffff' }}>{displayTitle}</strong> ({appId})
    </>
  );

  return (
    <>
      <ModalShell
        title={headerTitle}
        onClose={onClose}
        closeDisabled={saving}
        closeOnOverlayClick={false}
        closeOnEscape={false}
        containerClassName="achievements-modal"
        bodyClassName="achievements-modal-body"
      >
        {snapshot && (
          <div className="ach-summary">
            <div className="ach-summary-top">
              <span className="ach-summary-count">
                {effectiveUnlockedCount} / {totalCount} unlocked ({percent}%)
              </span>
              {snapshot.accounts.length > 0 && (
                <div className="ach-account-picker">
                  <label htmlFor="ach-account-select">Steam account</label>
                  <select
                    id="ach-account-select"
                    value={selectedAccountId ?? snapshot.accountId}
                    disabled={loading || saving}
                    onChange={(e) => {
                      const nextId = Number(e.target.value);
                      setSelectedAccountId(nextId);
                      void load(nextId);
                    }}
                  >
                    {snapshot.accounts.map((acct) => (
                      <option key={acct.accountId} value={acct.accountId}>
                        {acct.personaName}
                        {acct.isActive ? ' (Active)' : ''}
                      </option>
                    ))}
                  </select>
                </div>
              )}
            </div>
            <div
              className="ach-progress-bar"
              role="progressbar"
              aria-valuenow={percent}
              aria-valuemin={0}
              aria-valuemax={100}
            >
              <div className="ach-progress-fill" style={{ width: `${percent}%` }} />
            </div>
          </div>
        )}

        {snapshot && !snapshot.luaManaged && (
          <div className="ach-banner ach-banner--warn">
            Not managed by Lua: if you legitimately own this game on Steam, Valve servers may overwrite local
            achievement edits on the next launch.
          </div>
        )}

        {snapshot?.sources.mergedFromBackup && (
          <div className="ach-banner ach-banner--info">
            Note: recovered missing unlocks from Aether backup because Steam&apos;s local stats cache was incomplete.
          </div>
        )}

        {parsedError && (
          <div className={`ach-banner ${parsedError.code === 'NO_SCHEMA' ? 'ach-banner--info' : 'ach-banner--error'}`}>
            <strong>{parsedError.code === 'NO_SCHEMA' ? 'Schema not cached yet' : 'Error'}</strong>
            <p>{parsedError.text}</p>
          </div>
        )}

        {loading && <p className="ach-loading">Loading achievements cache...</p>}

        {snapshot && !loading && (
          <>
            <div className="ach-toolbar">
              <input
                type="search"
                className="home-search-input library-search-input ach-search"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Search achievements..."
                aria-label="Search achievements"
              />
              <div className="ach-bulk-actions">
                <button type="button" className="ach-small-btn" onClick={() => setRevealHidden((v) => !v)}>
                  {revealHidden ? 'Mask Hidden' : 'Show Hidden'}
                </button>
                <button
                  type="button"
                  className="ach-small-btn"
                  disabled={saving || (bulkAction !== 'unlockAll' && snapshot.achievements.every((a) => a.unlocked))}
                  onClick={handleUnlockAllClick}
                >
                  {bulkAction === 'unlockAll' ? 'Discard' : 'Unlock All'}
                </button>
                <button
                  type="button"
                  className="ach-small-btn"
                  disabled={saving || (bulkAction !== 'lockAll' && snapshot.achievements.every((a) => !a.unlocked))}
                  onClick={handleLockAllClick}
                >
                  {bulkAction === 'lockAll' ? 'Discard' : 'Lock All'}
                </button>
              </div>
            </div>

            <div className="ach-list-scroll">
              {unlockedItems.length > 0 && (
                <section className="ach-section">
                  <h4 className="ach-section-header">Unlocked Achievements ({unlockedItems.length})</h4>
                  <div className="ach-section-list">{unlockedItems.map(renderAchievementRow)}</div>
                </section>
              )}

              {lockedItems.length > 0 && (
                <section className="ach-section">
                  <h4 className="ach-section-header">Locked Achievements ({lockedItems.length})</h4>
                  <div className="ach-section-list">{lockedItems.map(renderAchievementRow)}</div>
                </section>
              )}

              {unlockedItems.length === 0 && lockedItems.length === 0 && (
                <p className="ach-empty">No achievements match the current filter.</p>
              )}
            </div>

            <div className="ach-footer">
              <div className="ach-footer-actions">
                <button
                  type="button"
                  className="ach-footer-btn ach-footer-btn--secondary"
                  disabled={saving}
                  onClick={onClose}
                >
                  Close
                </button>
                <button
                  type="button"
                  className="ach-footer-btn ach-footer-btn--primary"
                  disabled={saving || edits.length === 0}
                  onClick={() => void commitSave(false)}
                >
                  {saving ? 'Saving...' : 'Save'}
                </button>
              </div>
            </div>
          </>
        )}
      </ModalShell>

      {confirmSteamClose && (
        <SteamCloseConfirmModal
          onConfirm={() => {
            setConfirmSteamClose(false);
            void commitSave(true);
          }}
          onCancel={() => setConfirmSteamClose(false)}
        />
      )}
    </>
  );
};
