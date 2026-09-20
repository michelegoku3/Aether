import { TabType } from './Sidebar';
import { StoreView } from '../views/StoreView';
import { SettingsView, type SettingsGuard } from '../views/SettingsView';
import { AetherView } from '../views/AetherView';
import { LibraryView } from '../views/LibraryView';
import { HomeView } from '../views/HomeView';
import { LogView } from '../views/LogView';
import { DllStatusInfo } from '../types/ui';

export interface MainContentUpdates {
  dllAvailable: boolean;
  deskAvailable: boolean;
  deskVersion: string;
  dllIsTest: boolean;
  deskIsTest: boolean;
  onComplete: () => void;
}

export interface MainContentAppearance {
  useAlternativeGameCards: boolean;
  alternativeCardsOpacity: number;
  alternativeCardsFade: number;
  onRefreshCustomCss: () => Promise<void>;
  onCustomCssChange: (enabled: boolean) => void;
  onPreviewPersonalWallpaper: (enabled: boolean, opacity: number) => void;
  onPreviewAlternativeCards: (opacity: number, fade: number) => void;
}

export interface MainContentSettings {
  ready: boolean;
  revision: number;
  guardRef: { current: SettingsGuard | null };
  onMissingSteamPath: () => void;
  hubcapUsage: { usage: number; limit: number; hasKey: boolean };
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
}

export interface MainContentDll {
  status: DllStatusInfo;
  onChange: () => Promise<void>;
}

export interface MainContentProps {
  activeTab: TabType;
  updates: MainContentUpdates;
  appearance: MainContentAppearance;
  settings: MainContentSettings;
  dll: MainContentDll;
}

export const MainContent = ({
  activeTab,
  updates,
  appearance,
  settings,
  dll,
}: MainContentProps) => {
  const renderActiveTransientView = () => {
    if (activeTab === 'home') {
      return (
        <main className="main-content">
          <HomeView />
        </main>
      );
    }

    if (activeTab === 'aether') {
      return (
        <main className="main-content">
          <AetherView
            isUpdateAvailable={updates.dllAvailable}
            isDeskUpdateAvailable={updates.deskAvailable}
            deskVersion={updates.deskVersion}
            isDllUpdateTest={updates.dllIsTest}
            isDeskUpdateTest={updates.deskIsTest}
            onUpdateComplete={updates.onComplete}
            dllStatus={dll.status}
            onDllStatusChange={dll.onChange}
          />
        </main>
      );
    }

    if (activeTab === 'store' || activeTab === 'library' || activeTab === 'log' || activeTab === 'settings') {
      return null;
    }

    const title = activeTab === 'backup'
      ? 'Backup View'
      : 'Blank View';

    return (
      <main className="main-content">
        <div className="blank-canvas">
          <div style={{
            display: 'flex',
            width: '100%',
            height: '100%',
            alignItems: 'center',
            justifyContent: 'center',
            color: '#8f8f9e',
            fontSize: '24px',
            fontWeight: 'bold',
            letterSpacing: '1px'
          }}>
            {title}
          </div>
        </div>
      </main>
    );
  };

  return (
    <>
      {/* Store stays mounted so trending results, pagination state and resolved
          cover cache survive sidebar tab switches. */}
      <main
        className="main-content"
        style={{ display: activeTab === 'store' ? 'flex' : 'none' }}
        aria-hidden={activeTab !== 'store'}
      >
        <StoreView
          onRefreshUsage={settings.onRefreshUsage}
          settingsRevision={settings.revision}
          settingsReady={settings.ready}
          useAlternativeGameCards={appearance.useAlternativeGameCards}
          alternativeCardsOpacity={appearance.alternativeCardsOpacity}
          alternativeCardsFade={appearance.alternativeCardsFade}
        />
      </main>

      {/* Library stays mounted so search query, filter state, scroll position
          and loaded games survive tab switches. */}
      <main
        className="main-content"
        style={{ display: activeTab === 'library' ? 'flex' : 'none' }}
        aria-hidden={activeTab !== 'library'}
      >
        <LibraryView
          useAlternativeGameCards={appearance.useAlternativeGameCards}
          alternativeCardsOpacity={appearance.alternativeCardsOpacity}
          alternativeCardsFade={appearance.alternativeCardsFade}
        />
      </main>

      {/* Logs stay mounted so active source selection (Desk/DLL/UCO2/All),
          log level filter, search keyword and terminal scroll position persist. */}
      <main
        className="main-content"
        style={{ display: activeTab === 'log' ? 'flex' : 'none' }}
        aria-hidden={activeTab !== 'log'}
      >
        <LogView />
      </main>

      {/* Settings stay mounted so unsaved edits (e.g. the Steam path field)
          survive tab switches. Saving always merges over freshly loaded
          settings, so keeping it mounted cannot regress concurrent saves. */}
      <main
        className="main-content"
        style={{ display: activeTab === 'settings' ? 'flex' : 'none' }}
        aria-hidden={activeTab !== 'settings'}
      >
        <SettingsView
          hubcapUsage={settings.hubcapUsage}
          onRefreshUsage={settings.onRefreshUsage}
          onRefreshCustomCss={appearance.onRefreshCustomCss}
          onCustomCssChange={appearance.onCustomCssChange}
          onPreviewPersonalWallpaper={appearance.onPreviewPersonalWallpaper}
          onPreviewAlternativeCards={appearance.onPreviewAlternativeCards}
          onMissingSteamPath={settings.onMissingSteamPath}
          guardRef={settings.guardRef}
        />
      </main>

      {renderActiveTransientView()}
    </>
  );
};
