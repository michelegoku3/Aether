import { TabType } from './Sidebar';
import { StoreView } from './StoreView';
import { SettingsView } from './SettingsView';
import { AetherView } from './AetherView';
import { LibraryView } from './LibraryView';
import { HomeView } from './HomeView';

interface MainContentProps {
  activeTab: TabType;
  dllUpdateAvailable: boolean; // Passed down from App.tsx orchestrator
  deskUpdateAvailable: boolean; // Passed down from App.tsx orchestrator
  onUpdateComplete: () => void; // Passed down from App.tsx orchestrator
  hubcapUsage: { usage: number; limit: number; hasKey: boolean };
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
}

export const MainContent = ({ activeTab, dllUpdateAvailable, deskUpdateAvailable, onUpdateComplete, hubcapUsage, onRefreshUsage }: MainContentProps) => {
  // Route to the appropriate view based on the active tab
  if (activeTab === 'home') {
    return (
      <main className="main-content">
        <HomeView />
      </main>
    );
  }

  if (activeTab === 'store') {
    return (
      <main className="main-content">
        <StoreView onRefreshUsage={onRefreshUsage} />
      </main>
    );
  }

  if (activeTab === 'library') {
    return (
      <main className="main-content">
        <LibraryView />
      </main>
    );
  }

  if (activeTab === 'settings') {
    return (
      <main className="main-content">
        <SettingsView 
          hubcapUsage={hubcapUsage}
          onRefreshUsage={onRefreshUsage}
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
          onUpdateComplete={onUpdateComplete}
        />
      </main>
    );
  }

  // Fallback title for other blank pages
  const getTabTitle = () => {
    switch (activeTab) {
      case 'download': return 'Backup View';
      case 'log': return 'Log View';
      default: return 'Blank View';
    }
  };

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
          {getTabTitle()}
        </div>
      </div>
    </main>
  );
};
