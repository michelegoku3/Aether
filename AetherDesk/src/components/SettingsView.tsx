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
  
  const [statusMsg, setStatusMsg] = useState({ text: '', type: 'info' });
  const [validationStatus, setValidationStatus] = useState<'idle' | 'validating' | 'valid' | 'invalid'>('idle');

  // Load settings from the backend when the component mounts
  useEffect(() => {
    const loadSettings = async () => {
      try {
        const settings: any = await invoke('get_settings');
        if (settings) {
          setApiKey(settings.hubcap_api_key || '');
          setSteamPath(settings.steam_path || 'C:\\Program Files (x86)\\Steam');
          setActiveLibrary(settings.active_library || '');
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
    try {
      showStatus('Saving settings...', 'info');
      await invoke('save_settings', {
        settings: {
          hubcap_api_key: apiKey,
          steam_path: steamPath,
          active_library: activeLibrary
        }
      });
      showStatus('Settings saved successfully!', 'success');
      onRefreshUsage(apiKey);
    } catch (err: any) {
      showStatus(`Error during save: ${err}`, 'error');
    }
  };

  const handleValidateKey = async () => {
    if (!apiKey.trim()) {
      showStatus('Please enter an API key to validate first.', 'error');
      return;
    }
    
    setValidationStatus('validating');
    try {
      const isValid: any = await invoke('validate_hubcap_key', { apiKey });
      if (isValid) {
        setValidationStatus('valid');
        showStatus('Hubcap API key is valid and connected successfully!', 'success');
        onRefreshUsage(apiKey);
      } else {
        setValidationStatus('invalid');
        showStatus('Hubcap API key is invalid or expired.', 'error');
      }
    } catch (err: any) {
      setValidationStatus('invalid');
      showStatus(`Error validating API key: ${err}`, 'error');
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
            <span style={{ fontSize: '12px', color: '#8f8f9e', fontWeight: 'bold', marginLeft: '12px', whiteSpace: 'nowrap' }}>
              {hubcapUsage.hasKey ? `${hubcapUsage.usage}/${hubcapUsage.limit}` : 'N/A'}
            </span>
          </div>
          <div className="api-input-row">
            <input 
              type="password" 
              placeholder="Enter API key (e.g. smm_...)"
              value={apiKey}
              onChange={(e) => {
                setApiKey(e.target.value);
                setValidationStatus('idle'); // Reset validation status on change
              }}
              className="settings-input"
            />
            <button 
              type="button" 
              onClick={handleValidateKey}
              className={`validate-btn ${validationStatus}`}
              disabled={validationStatus === 'validating'}
            >
              {validationStatus === 'validating' ? 'Verifying...' :
               validationStatus === 'valid' ? 'Connected ✓' :
               validationStatus === 'invalid' ? 'Invalid ✗' : 'Verify Key'}
            </button>
          </div>
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

        {/* Steam Library Path Section */}
        <div className="settings-group">
          <label className="settings-label">Active Steam Library (Optional)</label>
          <p className="settings-desc">If your games are installed in a secondary library (e.g. D:\), specify the library path here (e.g. D:\SteamLibrary).</p>
          <input 
            type="text" 
            placeholder="Leave blank to use Steam's default library directory"
            value={activeLibrary}
            onChange={(e) => setActiveLibrary(e.target.value)}
            className="settings-input"
          />
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
