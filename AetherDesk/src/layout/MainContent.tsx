import { TabType } from './Sidebar';
import { StoreView } from '../views/StoreView';
import { SettingsView } from '../views/SettingsView';
import { AetherView } from '../views/AetherView';
import { LibraryView } from '../views/LibraryView';
import { HomeView } from '../views/HomeView';
import { DllStatusInfo } from '../types/ui';

interface MainContentProps {
  activeTab: TabType;
  dllUpdateAvailable: boolean; // Passed down from App.tsx orchestrator
  deskUpdateAvailable: boolean; // Passed down from App.tsx orchestrator
  dllUpdateIsTest: boolean;     // Whether the DLL update is a test build (red)
  deskUpdateIsTest: boolean;    // Whether the desk update is a test build (red)
  onUpdateComplete: () => void; // Passed down from App.tsx orchestrator
  hubcapUsage: { usage: number; limit: number; hasKey: boolean };
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
  dllStatus: DllStatusInfo;
  onDllStatusChange: () => Promise<void>;
  onRefreshCustomCss: () => Promise<void>;
  onCustomCssChange: (enabled: boolean) => void;
  onPreviewPersonalWallpaper: (enabled: boolean, opacity: number) => void;
  onPreviewAlternativeCards: (opacity: number, fade: number) => void;
  settingsRevision: number;
  useAlternativeGameCards: boolean;
  alternativeCardsOpacity: number;
  alternativeCardsFade: number;
}

export const MainContent = ({ activeTab, dllUpdateAvailable, deskUpdateAvailable, dllUpdateIsTest, deskUpdateIsTest, onUpdateComplete, hubcapUsage, onRefreshUsage, dllStatus, onDllStatusChange, onRefreshCustomCss, onCustomCssChange, onPreviewPersonalWallpaper, onPreviewAlternativeCards, settingsRevision, useAlternativeGameCards, alternativeCardsOpacity, alternativeCardsFade }: MainContentProps) => {
  const renderActiveNonStoreView = () => {
    if (activeTab === 'store') {
      return null;
    }

    if (activeTab === 'home') {
      return (
        <main className="main-content">
          <HomeView />
        </main>
      );
    }

    if (activeTab === 'library') {
      return (
        <main className="main-content">
          <LibraryView
            useAlternativeGameCards={useAlternativeGameCards}
            alternativeCardsOpacity={alternativeCardsOpacity}
            alternativeCardsFade={alternativeCardsFade}
          />
        </main>
      );
    }

    if (activeTab === 'settings') {
      return (
        <main className="main-content">
          <SettingsView
            hubcapUsage={hubcapUsage}
            onRefreshUsage={onRefreshUsage}
            onRefreshCustomCss={onRefreshCustomCss}
            onCustomCssChange={onCustomCssChange}
            onPreviewPersonalWallpaper={onPreviewPersonalWallpaper}
            onPreviewAlternativeCards={onPreviewAlternativeCards}
          />
        </main>
      );
    }

    if (activeTab === 'aether') {
      return (
        <main className="main-content">
          <AetherView
            isUpdateAvailable={dllUpdateAvailable}
            isDeskUpdateAvailable={deskUpdateAvailable}
            isDllUpdateTest={dllUpdateIsTest}
            isDeskUpdateTest={deskUpdateIsTest}
            onUpdateComplete={onUpdateComplete}
            dllStatus={dllStatus}
            onDllStatusChange={onDllStatusChange}
          />
        </main>
      );
    }

    const title = activeTab === 'backup'
      ? 'Backup View'
      : activeTab === 'log'
        ? 'Logs View'
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
          cover cache survive sidebar tab switches. It is hidden, not unmounted. */}
      <main
        className="main-content"
        style={{ display: activeTab === 'store' ? 'flex' : 'none' }}
        aria-hidden={activeTab !== 'store'}
      >
        <StoreView
          onRefreshUsage={onRefreshUsage}
          isActive={activeTab === 'store'}
          settingsRevision={settingsRevision}
          useAlternativeGameCards={useAlternativeGameCards}
          alternativeCardsOpacity={alternativeCardsOpacity}
          alternativeCardsFade={alternativeCardsFade}
        />
      </main>
      {renderActiveNonStoreView()}
    </>
  );
};
