export type TabType = 'aether' | 'home' | 'store' | 'library' | 'backup' | 'settings' | 'log';

interface SidebarProps {
  activeTab: TabType;
  onTabChange: (tab: TabType) => void;
  onRestartSteam: () => void;
  dllUpdateAvailable: boolean; // Received from parent (App.tsx)
  updateIsTest: boolean;        // Whether the shown update is a test build (red)
}

export const Sidebar = ({ activeTab, onTabChange, onRestartSteam, dllUpdateAvailable, updateIsTest }: SidebarProps) => {
  return (
    <aside className="sidebar">
      {/* TOP NAVIGATION SECTION */}
      <div className="sidebar-top">
        {/* Brand Header */}
        <div className="sidebar-brand">
          <div className="brand-icon">Æ</div>
          <span className="brand-name">AETHER</span>
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

        {/* Backup */}
        <button
          disabled={true}
          title="Backup is not available yet"
          className="nav-item"
        >
          Backup
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
            <span
              className={`sidebar-update-dot${updateIsTest ? ' test' : ''}`}
              title={updateIsTest ? 'Aether TEST update available!' : 'AetherDLL update available!'}
            ></span>
          )}
        </button>

        {/* Settings */}
        <button
          onClick={() => onTabChange('settings')}
          className={`nav-item ${activeTab === 'settings' ? 'active' : ''}`}
        >
          Settings
        </button>

        {/* Logs */}
        <button
          onClick={() => onTabChange('log')}
          className={`nav-item ${activeTab === 'log' ? 'active' : ''}`}
          title="Session Logs & Real-Time Terminal Console"
        >
          Logs
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
