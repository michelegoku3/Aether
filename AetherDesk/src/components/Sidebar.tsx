export type TabType = 'aether' | 'home' | 'store' | 'library' | 'download' | 'settings' | 'log';

interface SidebarProps {
  activeTab: TabType;
  onTabChange: (tab: TabType) => void;
  onRestartSteam: () => void;
  dllUpdateAvailable: boolean; // Received from parent (App.tsx)
}

export const Sidebar = ({ activeTab, onTabChange, onRestartSteam, dllUpdateAvailable }: SidebarProps) => {
  return (
    <aside className="sidebar">
      {/* TOP NAVIGATION SECTION */}
      <div className="sidebar-top">
        {/* Brand Header */}
        <div className="sidebar-brand">
          <div className="brand-icon">Æ</div>
          <span className="brand-name">AetherDesk</span>
        </div>

        {/* Separator Line */}
        <div className="separator brand-separator"></div>

        {/* Home */}
        <button
          onClick={() => onTabChange('home')}
          className={`nav-item ${activeTab === 'home' ? 'active' : ''}`}
        >
          Home
        </button>

        {/* Store */}
        <button
          onClick={() => onTabChange('store')}
          className={`nav-item ${activeTab === 'store' ? 'active' : ''}`}
        >
          Store
        </button>

        {/* Library */}
        <button
          onClick={() => onTabChange('library')}
          className={`nav-item ${activeTab === 'library' ? 'active' : ''}`}
        >
          Library
        </button>

        {/* Download */}
        <button
          onClick={() => onTabChange('download')}
          className={`nav-item ${activeTab === 'download' ? 'active' : ''}`}
        >
          Download
        </button>

        {/* Separator Line */}
        <div className="separator"></div>

        {/* Aether (Moved here: above settings, below the separator line) */}
        <button
          onClick={() => onTabChange('aether')}
          className={`nav-item ${activeTab === 'aether' ? 'active' : ''}`}
          style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}
        >
          <span>Aether</span>
          {dllUpdateAvailable && (
            <span className="sidebar-update-dot" title="AetherDLL update available!"></span>
          )}
        </button>

        {/* Settings */}
        <button
          onClick={() => onTabChange('settings')}
          className={`nav-item ${activeTab === 'settings' ? 'active' : ''}`}
        >
          Settings
        </button>

        {/* Log */}
        <button
          onClick={() => onTabChange('log')}
          className={`nav-item ${activeTab === 'log' ? 'active' : ''}`}
        >
          Log
        </button>
      </div>

      {/* Spacing is handled automatically by CSS flexbox spacing on .sidebar */}

      {/* BOTTOM SECTION */}
      <div className="sidebar-footer">
        {/* Separator Line */}
        <div className="separator"></div>

        {/* Restart Steam Button (Action, not a tab) */}
        <button
          onClick={onRestartSteam}
          className="nav-item restart-steam-btn"
        >
          Restart Steam
        </button>
      </div>
    </aside>
  );
};
export type { SidebarProps };
