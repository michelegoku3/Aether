import { TabType } from './Sidebar';

interface MainContentProps {
  activeTab: TabType;
}

export const MainContent = ({ activeTab }: MainContentProps) => {
  // Translate the activeTab into a readable title inside the blank canvas
  const getTabTitle = () => {
    switch (activeTab) {
      case 'aether': return 'Aether View';
      case 'home': return 'Home View';
      case 'store': return 'Store View';
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
        {/* Keeping the canvas clean and white, with a subtle debug title */}
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
