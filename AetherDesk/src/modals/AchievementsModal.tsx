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

type FilterTab = 'all' | 'unlocked' | 'locked' | 'hidden';

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
  const [search, setSearch] = useState('');
  const [filterTab, setFilterTab] = useState<FilterTab>('all');
  const [revealHidden, setRevealHidden] = useState(false);
  const [brokenImages, setBrokenImages] = useState<Record<string, true>>({});
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
      return next;
    });
  };

  const setAll = (targetUnlocked: boolean) => {
    if (!snapshot) return;
    const next: Record<number, boolean> = {};
    for (const item of snapshot.achievements) {
      if (targetUnlocked !== item.unlocked) {
        next[item.id] = targetUnlocked;
      }
    }
    setPending(next);
  };

  const matchesSearch = useCallback(
    (item: AchievementItemView, effectiveUnlocked: boolean) => {
      const q = search.trim().toLowerCase();
      if (!q) return true;
      const isMasked = item.hidden && !effectiveUnlocked && !revealHidden;
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

  const { unlockedItems, lockedItems } = useMemo(() => {
    const unlockedList: AchievementItemView[] = [];
    const lockedList: AchievementItemView[] = [];
    if (!snapshot) return { unlockedItems: unlockedList, lockedItems: lockedList };

    for (const item of snapshot.achievements) {
      const effectiveUnlocked = pending[item.id] ?? item.unlocked;
      if (filterTab === 'unlocked' && !effectiveUnlocked) continue;
      if (filterTab === 'locked' && effectiveUnlocked) continue;
      if (filterTab === 'hidden' && !item.hidden) continue;
      if (!matchesSearch(item, effectiveUnlocked)) continue;

      if (effectiveUnlocked) {
        unlockedList.push(item);
      } else {
        lockedList.push(item);
      }
    }
    return { unlockedItems: unlockedList, lockedItems: lockedList };
  }, [snapshot, pending, filterTab, matchesSearch]);

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
    const isMasked = item.hidden && !effectiveUnlocked && !revealHidden;

    const preferredUrl = effectiveUnlocked ? item.iconUrl : (item.iconGrayUrl ?? item.iconUrl);
    const activeIconUrl = preferredUrl && !brokenImages[preferredUrl] ? preferredUrl : null;
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
              loading="lazy"
              onError={() => setBrokenImages((prev) => ({ ...prev, [preferredUrl!]: true }))}
            />
          ) : (
            <TrophyIcon size={24} />
          )}
        </div>

        <div className="ach-meta">
          <div className="ach-title-line">
            <span className="ach-name">{displayName}</span>
            {item.hidden && <span className="ach-badge ach-badge--hidden">Hidden</span>}
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
  const headerTitle = `${snapshot?.gameName ?? gameName} — Achievements`;

  return (
    <>
      <ModalShell
        title={headerTitle}
        onClose={onClose}
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
                className="ach-search"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Search achievements..."
                aria-label="Search achievements"
              />
              <div className="ach-tabs" role="tablist" aria-label="Achievement filter">
                {(['all', 'unlocked', 'locked', 'hidden'] as FilterTab[]).map((tab) => (
                  <button
                    key={tab}
                    type="button"
                    role="tab"
                    aria-selected={filterTab === tab}
                    className={`ach-tab-btn${filterTab === tab ? ' active' : ''}`}
                    onClick={() => setFilterTab(tab)}
                  >
                    {tab[0].toUpperCase() + tab.slice(1)}
                  </button>
                ))}
              </div>
              <div className="ach-bulk-actions">
                <button type="button" className="ach-small-btn" onClick={() => setRevealHidden((v) => !v)}>
                  {revealHidden ? 'Mask hidden' : 'Reveal hidden'}
                </button>
                <button type="button" className="ach-small-btn" disabled={saving} onClick={() => setAll(true)}>
                  Unlock all
                </button>
                <button type="button" className="ach-small-btn" disabled={saving} onClick={() => setAll(false)}>
                  Lock all
                </button>
                <button
                  type="button"
                  className="ach-small-btn"
                  disabled={saving || edits.length === 0}
                  onClick={() => setPending({})}
                >
                  Discard
                </button>
              </div>
            </div>

            <div className="ach-list-scroll">
              {unlockedItems.length > 0 && (
                <section className="ach-section">
                  <h4 className="ach-section-header">Unlocked ({unlockedItems.length})</h4>
                  <div className="ach-section-list">{unlockedItems.map(renderAchievementRow)}</div>
                </section>
              )}

              {lockedItems.length > 0 && (
                <section className="ach-section">
                  <h4 className="ach-section-header">Locked ({lockedItems.length})</h4>
                  <div className="ach-section-list">{lockedItems.map(renderAchievementRow)}</div>
                </section>
              )}

              {unlockedItems.length === 0 && lockedItems.length === 0 && (
                <p className="ach-empty">No achievements match the current filter.</p>
              )}
            </div>

            <div className="ach-footer">
              <div className="ach-footer-status">
                {edits.length > 0 ? (
                  <span className="ach-dirty-count">
                    {edits.length} pending edit{edits.length === 1 ? '' : 's'}
                  </span>
                ) : (
                  <span className="ach-schema-meta">
                    Schema v{snapshot.schemaVersion} •{' '}
                    {snapshot.schemaSource === 'steamCache' ? 'Steam cache' : 'Aether backup'}
                  </span>
                )}
              </div>
              <div className="ach-footer-actions">
                <button
                  type="button"
                  className="uninstall-btn uninstall-btn-secondary"
                  disabled={saving}
                  onClick={onClose}
                >
                  Close
                </button>
                <button
                  type="button"
                  className="ach-save-btn"
                  disabled={saving || edits.length === 0}
                  onClick={() => void commitSave(false)}
                >
                  {saving ? 'Saving...' : 'Save changes'}
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
