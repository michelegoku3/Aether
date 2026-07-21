import { useState } from 'react';

export const AetherView = () => {
  // Mock states to manage button activations for the UI layout demonstration
  const [isDllInstalled, setIsDllInstalled] = useState(false);
  const [isSteamBlocked, setIsSteamBlocked] = useState(false);
  const [isUpdateAvailable, setIsUpdateAvailable] = useState(false);

  return (
    <div className="aether-view">
      
      {/* Big Uppercase Title at the top */}
      <h1 className="aether-title">AETHER</h1>
      
      {/* SECTION 1: AetherDesk */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">AetherDesk</span>
          <span className="panel-meta">v1.0.0</span>
        </div>
        <div className="panel-actions">
          <button 
            className="panel-btn" 
            disabled={!isUpdateAvailable}
          >
            Update
          </button>
        </div>
      </div>

      {/* SECTION 2: AetherDLL */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">AetherDLL</span>
          <span className="panel-meta">{isDllInstalled ? 'v2.4.1' : 'N/A'}</span>
        </div>
        <div className="panel-actions">
          {/* Install/Update Button: Active if DLL is NOT installed, or if updates are available */}
          <button 
            onClick={() => setIsDllInstalled(true)}
            className="panel-btn"
            disabled={isDllInstalled && !isUpdateAvailable}
          >
            {isDllInstalled ? 'Update' : 'Install'}
          </button>

          {/* Uninstall Button: Active ONLY if DLL is detected/installed */}
          <button 
            onClick={() => setIsDllInstalled(false)}
            className="panel-btn"
            disabled={!isDllInstalled}
          >
            Uninstall
          </button>
        </div>
      </div>

      {/* SECTION 3: Steam */}
      <div className="aether-panel">
        <div className="panel-header">
          <span className="panel-title">Steam</span>
        </div>
        <div className="panel-actions">
          {/* Block / Unlock Update Button */}
          <button 
            onClick={() => setIsSteamBlocked(!isSteamBlocked)}
            className="panel-btn"
          >
            {isSteamBlocked ? 'Unlock Update' : 'Block Update'}
          </button>

          {/* Reset Path Button */}
          <button 
            className="panel-btn"
          >
            Reset Path
          </button>
        </div>
      </div>

    </div>
  );
};
