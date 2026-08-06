import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface SettingsViewProps {
  hubcapUsage: { usage: number; limit: number; hasKey: boolean };
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
}

export const SettingsView = ({ hubcapUsage, onRefreshUsage }: SettingsViewProps) => {
  const [apiKey, setApiKey] = useState('');
  const [steamPath, setSteamPath] = useState('C:\\Program Files (x86)\\Steam');
  const [activeLibrary, setActiveLibrary] = useState('');
  const [showStoreDlcs, setShowStoreDlcs] = useState(false);

  // Raw settings as loaded from the backend. Saving always spreads this object
  // back, so fields owned by other flows (e.g. `antivirus_exclusion_done`) are
  // never silently reset by a settings save.
  const [rawSettings, setRawSettings] = useState<Record<string, any>>({});

  const [statusMsg, setStatusMsg] = useState({ text: '', type: 'info' });

  // Load settings from the backend when the component mounts
  useEffect(() => {
    const loadSettings = async () => {
      try {
        const settings: any = await invoke('get_settings');
        if (settings) {
          setRawSettings(settings);
          setApiKey(settings.hubcap_api_key || '');
          setSteamPath(settings.steam_path || 'C:\\Program Files (x86)\\Steam');
          setActiveLibrary(settings.active_library || '');
          setShowStoreDlcs(Boolean(settings.show_store_dlcs));
        }
      } catch (err: any) {
        showStatus(`Error loading settings: ${err}`, 'error');
      }
    };
    loadSettings();
    onRefreshUsage();
  }, []);

  const showStatus = (text: string, type: 'info' | 'success' | 'error') => {
    setStatusMsg({ text, type });
    setTimeout(() => setStatusMsg({ text: '', type: 'info' }), 6000);
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    
    // Validazione API key se presente
    if (apiKey.trim()) {
      showStatus('Validating API key...', 'info');
      try {
        const isValid: any = await invoke('validate_hubcap_key', { apiKey });
        if (!isValid) {
          showStatus('Invalid API key. Settings not saved.', 'error');
          return;
        }
      } catch (err: any) {
        showStatus(`API key validation failed: ${err}`, 'error');
        return;
      }
    }
    
    // Salvataggio
    try {
      showStatus('Saving settings...', 'info');
      await invoke('save_settings', {
        settings: {
          ...rawSettings,
          hubcap_api_key: apiKey,
          steam_path: steamPath,
          active_library: activeLibrary,
          show_store_dlcs: showStoreDlcs
        }
      });
      showStatus('Settings saved successfully!', 'success');
      onRefreshUsage(apiKey);
    } catch (err: any) {
      showStatus(`Error during save: ${err}`, 'error');
    }
  };

  return (
    <div className="settings-view">
      <div className="settings-header">
        <h1 className="settings-title">Settings</h1>
        <p className="settings-subtitle">Manage system configurations, Hubcap API keys, and Steam injection paths.</p>
      </div>

      <div className="settings-separator"></div>

      {statusMsg.text && (
        <div className={`settings-alert ${statusMsg.type}`}>
          {statusMsg.text}
        </div>
      )}

      <form onSubmit={handleSave} className="settings-form">
        {/* Hubcap API Key Section */}
        <div className="settings-group">
          <label className="settings-label">Hubcap API Key</label>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
            <p className="settings-desc">Enter your hubcapmanifest.com API key to unlock database lookups and downloads.</p>
            {hubcapUsage.hasKey && (
              <span style={{ fontSize: '12px', color: '#8f8f9e', fontWeight: 'bold', marginLeft: '12px', whiteSpace: 'nowrap' }}>
                {hubcapUsage.usage}/{hubcapUsage.limit}
              </span>
            )}
          </div>
          <input 
            type="password" 
            placeholder="Enter API key (e.g. smm_...)"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            className="settings-input"
          />
        </div>

        {/* Steam Path Section */}
        <div className="settings-group">
          <label className="settings-label">Steam Installation Path</label>
          <p className="settings-desc">The main directory path where Steam is installed on your PC (required for DLLs and configuration installation).</p>
          <input 
            type="text" 
            placeholder="C:\Program Files (x86)\Steam"
            value={steamPath}
            onChange={(e) => setSteamPath(e.target.value)}
            className="settings-input"
          />
        </div>

        <div className="settings-separator"></div>

        {/* Store Section */}
        <div className="settings-group">
          <label className="settings-label">Store</label>
          <div className="settings-toggle-row" title="Show downloadable and non-downloadable add-ons (DLC) in store search results">
            <span className="settings-toggle-text">Show DLCs in the store</span>
            <label className="version-switch">
              <input
                type="checkbox"
                checked={showStoreDlcs}
                onChange={(e) => setShowStoreDlcs(e.target.checked)}
              />
              <span></span>
            </label>
          </div>
        </div>

        <div className="settings-separator"></div>

        <div className="form-actions">
          <button type="submit" className="save-settings-btn">
            Save Settings
          </button>
        </div>
      </form>
    </div>
  );
};
