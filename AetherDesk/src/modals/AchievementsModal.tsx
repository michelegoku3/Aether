import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalShell } from '../ui/ModalShell';
import { SteamCloseConfirmModal } from './SteamCloseConfirmModal';
import { toErrorMessage } from '../util/errors';
import type {
  AchievementChange,
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

export const AchievementsModal = ({ appId, gameName, onClose, onStatus }: AchievementsModalProps) => {
  const [snapshot, setSnapshot] = useState<GameAchievementsSnapshot | null>(null);
  const [pending, setPending] = useState<Record<number, boolean>>({});
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

  const edits: AchievementChange[] = Object.entries(pending).map(([id, unlocked]) => ({
    id: Number(id),
    unlocked,
  }));

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

  return (
    <>
      <ModalShell title={`${gameName} — Achievements`} onClose={onClose} containerClassName="achievements-modal">
        {loading && <p>Loading achievements...</p>}
        {error && <p className="achievements-error">{error}</p>}
        {snapshot && (
          <div>
            {snapshot.achievements.map((item) => {
              const effectiveUnlocked = pending[item.id] ?? item.unlocked;
              return (
                <label key={item.id}>
                  <input
                    type="checkbox"
                    checked={effectiveUnlocked}
                    onChange={() =>
                      setPending((prev) => {
                        const next = { ...prev };
                        if (!effectiveUnlocked === item.unlocked) {
                          delete next[item.id];
                        } else {
                          next[item.id] = !effectiveUnlocked;
                        }
                        return next;
                      })
                    }
                  />
                  <span>{item.name}</span>
                </label>
              );
            })}
            <button type="button" disabled={saving || edits.length === 0} onClick={() => void commitSave(false)}>
              Save changes
            </button>
          </div>
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
