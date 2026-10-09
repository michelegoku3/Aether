import { useMemo } from 'react';
import { Sidebar } from './layout/Sidebar';
import { MainContent } from './layout/MainContent';
import type {
  MainContentAppearance,
  MainContentDll,
  MainContentSettings,
  MainContentUpdates,
} from './layout/MainContent';
import { LibraryGamesProvider } from './hooks/useLibraryGames';
import { SteamPathWarningModal } from './modals/SteamPathWarningModal';
import { UnsavedChangesModal } from './modals/UnsavedChangesModal';
import { useAppBootstrap } from './app/useAppBootstrap';
import { useAppearanceRuntime } from './app/useAppearanceRuntime';
import { useDllRuntime } from './app/useDllRuntime';
import { useHubcapUsage } from './app/useHubcapUsage';
import { useSteamRuntime } from './app/useSteamRuntime';
import {
  useUnsavedNavigationGuard,
  useWindowCloseGuard,
} from './app/useUnsavedNavigationGuard';
import { useUpdateCoordinator } from './app/useUpdateCoordinator';

/** Application composition root. Domain workflows live in focused app hooks. */
export default function App() {
  const dll = useDllRuntime();
  const appearance = useAppearanceRuntime({ onSettingsApplied: dll.checkStatus });
  const updates = useUpdateCoordinator();
  const hubcap = useHubcapUsage();
  const navigation = useUnsavedNavigationGuard();

  useAppBootstrap({
    loadDeskVersion: updates.loadDeskVersion,
    refreshAppearance: appearance.refresh,
    refreshHubcapUsage: hubcap.refresh,
    checkDeskUpdates: updates.checkDeskUpdates,
    checkDllUpdates: updates.checkDllUpdates,
    warnIfSteamPathMissing: navigation.warnIfSteamPathMissing,
  });
  useWindowCloseGuard(
    navigation.settingsGuardRef,
    navigation.beginPendingClose,
  );
  const steam = useSteamRuntime();

  // MainContent is memoized: these projections preserve object identity when
  // unrelated App state (for example Steam runtime state) changes.
  const updatesGroup = useMemo<MainContentUpdates>(() => ({
    dllAvailable: updates.dllUpdateAvailable,
    deskAvailable: updates.deskUpdateAvailable,
    deskVersion: updates.deskVersion,
    dllIsTest: updates.dllUpdateIsTest,
    deskIsTest: updates.deskUpdateIsTest,
    onComplete: updates.checkAllUpdates,
  }), [
    updates.dllUpdateAvailable,
    updates.deskUpdateAvailable,
    updates.deskVersion,
    updates.dllUpdateIsTest,
    updates.deskUpdateIsTest,
    updates.checkAllUpdates,
  ]);

  const appearanceGroup = useMemo<MainContentAppearance>(() => ({
    useAlternativeGameCards: appearance.useAlternativeGameCards,
    alternativeCardsOpacity: appearance.alternativeCardsOpacity,
    alternativeCardsFade: appearance.alternativeCardsFade,
    onRefreshCustomCss: appearance.refresh,
    onCustomCssChange: appearance.changeCustomCss,
    onPreviewPersonalWallpaper: appearance.previewPersonalWallpaper,
    onPreviewAlternativeCards: appearance.previewAlternativeCards,
  }), [
    appearance.useAlternativeGameCards,
    appearance.alternativeCardsOpacity,
    appearance.alternativeCardsFade,
    appearance.refresh,
    appearance.changeCustomCss,
    appearance.previewPersonalWallpaper,
    appearance.previewAlternativeCards,
  ]);

  const settingsGroup = useMemo<MainContentSettings>(() => ({
    ready: appearance.settingsReady,
    revision: appearance.settingsRevision,
    guardRef: navigation.settingsGuardRef,
    onMissingSteamPath: navigation.openSteamPathWarning,
    hubcapUsage: hubcap.usage,
    onRefreshUsage: hubcap.refresh,
  }), [
    appearance.settingsReady,
    appearance.settingsRevision,
    navigation.settingsGuardRef,
    navigation.openSteamPathWarning,
    hubcap.usage,
    hubcap.refresh,
  ]);

  const dllGroup = useMemo<MainContentDll>(() => ({
    status: dll.status,
    onChange: dll.checkStatus,
  }), [dll.status, dll.checkStatus]);

  return (
    <LibraryGamesProvider>
      <div className="app-container">
        <Sidebar
          activeTab={navigation.activeTab}
          onTabChange={navigation.changeTab}
          onSteamAction={steam.runAction}
          steamRunning={steam.running}
          steamBusy={steam.busy}
          dllUpdateAvailable={updates.dllUpdateAvailable || updates.deskUpdateAvailable}
          updateIsTest={updates.dllUpdateIsTest || updates.deskUpdateIsTest}
        />

        <MainContent
          activeTab={navigation.activeTab}
          updates={updatesGroup}
          appearance={appearanceGroup}
          settings={settingsGroup}
          dll={dllGroup}
        />

        {navigation.showUnsavedModal && (
          <UnsavedChangesModal
            busy={navigation.unsavedBusy}
            onSave={() => void navigation.saveUnsaved()}
            onDiscard={navigation.discardUnsaved}
            onCancel={navigation.cancelUnsaved}
          />
        )}

        {navigation.showSteamPathWarning && (
          <SteamPathWarningModal
            onGoToSettings={navigation.goToSettings}
            onClose={navigation.closeSteamPathWarning}
          />
        )}
      </div>
    </LibraryGamesProvider>
  );
}
