import { TabType } from './Sidebar';
import { StoreView } from './StoreView';
import { SettingsView } from './SettingsView';
import { AetherView } from './AetherView';

interface MainContentProps {
  activeTab: TabType;
  dllUpdateAvailable: boolean; // Passed down from App.tsx orchestrator
  deskUpdateAvailable: boolean; // Passed down from App.tsx orchestrator
  onUpdateComplete: () => void; // Passed down from App.tsx orchestrator
}

export const MainContent = ({ activeTab, dllUpdateAvailable, deskUpdateAvailable, onUpdateComplete }: MainContentProps) => {
  // Route to the appropriate view based on the active tab
  if (activeTab === 'store') {
    return (
      <main className="main-content">
        <StoreView />
      </main>
    );
  }

  if (activeTab === 'settings') {
    return (
      <main className="main-content">
        <SettingsView />
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
      case 'home': return 'Home View';
      case 'library': return 'Library View';
      case 'download': return 'Download View';
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
