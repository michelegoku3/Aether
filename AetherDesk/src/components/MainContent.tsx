import { TabType } from './Sidebar';
import { StoreView } from './StoreView';

interface MainContentProps {
  activeTab: TabType;
}

export const MainContent = ({ activeTab }: MainContentProps) => {
  // If the active tab is the store, render our modular StoreView component
  if (activeTab === 'store') {
    return (
      <main className="main-content">
        <StoreView />
      </main>
    );
  }

  // Fallback title for other blank pages
  const getTabTitle = () => {
    switch (activeTab) {
      case 'aether': return 'Aether View';
      case 'home': return 'Home View';
      case 'library': return 'Library View';
      case 'download': return 'Download View';
      case 'settings': return 'Settings View';
      case 'log': return 'Log View';
      case 'restart': return 'Restart Steam View';
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
