import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MutableRefObject,
} from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { hasValidSteamPath } from '../hooks/useSettings';
import type { TabType } from '../layout/Sidebar';
import type { SettingsGuard } from '../views/SettingsView';

type PendingAction = { type: 'tab'; tab: TabType } | { type: 'close' };

/** Owns tab navigation, Settings dirty prompts and the Steam-path warning. */
export function useUnsavedNavigationGuard() {
  const [activeTab, setActiveTab] = useState<TabType>('home');
  const [showSteamPathWarning, setShowSteamPathWarning] = useState(false);
  const [showUnsavedModal, setShowUnsavedModal] = useState(false);
  const [unsavedBusy, setUnsavedBusy] = useState(false);
  const settingsGuardRef = useRef<SettingsGuard | null>(null);
  const pendingActionRef = useRef<PendingAction | null>(null);

  const warnIfSteamPathMissing = useCallback(async () => {
    if (!(await hasValidSteamPath())) setShowSteamPathWarning(true);
  }, []);

  const changeTab = useCallback((tab: TabType) => {
    if (showUnsavedModal) return;
    if (activeTab === 'settings' && tab !== 'settings') {
      if (settingsGuardRef.current?.isDirty()) {
        pendingActionRef.current = { type: 'tab', tab };
        setShowUnsavedModal(true);
        return;
      }
      void warnIfSteamPathMissing();
    }
    setActiveTab(tab);
  }, [activeTab, showUnsavedModal, warnIfSteamPathMissing]);

  const completePendingAction = useCallback(async (pending: PendingAction) => {
    if (pending.type === 'tab') {
      setActiveTab(pending.tab);
      void warnIfSteamPathMissing();
      return;
    }
    try {
      await invoke('force_close_window');
    } catch (error) {
      console.error('Failed to close window:', error);
    }
  }, [warnIfSteamPathMissing]);

  const saveUnsaved = useCallback(async () => {
    if (unsavedBusy) return;
    setUnsavedBusy(true);
    try {
      const saved = await settingsGuardRef.current?.save();
      if (!saved) return;
      const pending = pendingActionRef.current;
      pendingActionRef.current = null;
      setShowUnsavedModal(false);
      if (pending) await completePendingAction(pending);
    } finally {
      setUnsavedBusy(false);
    }
  }, [completePendingAction, unsavedBusy]);

  const discardUnsaved = useCallback(() => {
    if (unsavedBusy) return;
    const pending = pendingActionRef.current;
    pendingActionRef.current = null;
    setShowUnsavedModal(false);
    settingsGuardRef.current?.discard();
    if (pending) void completePendingAction(pending);
  }, [completePendingAction, unsavedBusy]);

  const cancelUnsaved = useCallback(() => {
    if (unsavedBusy) return;
    pendingActionRef.current = null;
    setShowUnsavedModal(false);
  }, [unsavedBusy]);

  const beginPendingClose = useCallback(() => {
    pendingActionRef.current = { type: 'close' };
    setShowUnsavedModal(true);
  }, []);

  const openSteamPathWarning = useCallback(() => setShowSteamPathWarning(true), []);
  const closeSteamPathWarning = useCallback(() => setShowSteamPathWarning(false), []);
  const goToSettings = useCallback(() => {
    setShowSteamPathWarning(false);
    setActiveTab('settings');
  }, []);

  return {
    activeTab,
    settingsGuardRef,
    showSteamPathWarning,
    showUnsavedModal,
    unsavedBusy,
    changeTab,
    warnIfSteamPathMissing,
    beginPendingClose,
    openSteamPathWarning,
    closeSteamPathWarning,
    goToSettings,
    saveUnsaved,
    discardUnsaved,
    cancelUnsaved,
  };
}

/** Registers the native close interception after startup hydration is wired. */
export function useWindowCloseGuard(
  settingsGuardRef: MutableRefObject<SettingsGuard | null>,
  beginPendingClose: () => void,
) {
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void getCurrentWindow().onCloseRequested((event) => {
      if (settingsGuardRef.current?.isDirty()) {
        event.preventDefault();
        beginPendingClose();
      }
    }).then((off) => { unlisten = off; });
    return () => unlisten?.();
  }, [beginPendingClose, settingsGuardRef]);
}
