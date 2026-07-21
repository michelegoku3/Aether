import { TabType } from './Sidebar';
import { StoreView } from './StoreView';
import { SettingsView } from './SettingsView';
import { AetherView } from './AetherView';

interface MainContentProps {
  activeTab: TabType;
}

export const MainContent = ({ activeTab }: MainContentProps) => {
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
        <AetherView />
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
