import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

export const SettingsView = () => {
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
        showStatus(`Errore nel caricamento delle impostazioni: ${err}`, 'error');
      }
    };
    loadSettings();
  }, []);

  const showStatus = (text: string, type: 'info' | 'success' | 'error') => {
    setStatusMsg({ text, type });
    setTimeout(() => setStatusMsg({ text: '', type: 'info' }), 6000);
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      showStatus('Salvataggio in corso...', 'info');
      await invoke('save_settings', {
        settings: {
          hubcap_api_key: apiKey,
          steam_path: steamPath,
          active_library: activeLibrary
        }
      });
      showStatus('Impostazioni salvate con successo!', 'success');
    } catch (err: any) {
      showStatus(`Errore durante il salvataggio: ${err}`, 'error');
    }
  };

  const handleValidateKey = async () => {
    if (!apiKey.trim()) {
      showStatus('Inserisci prima una chiave API da convalidare.', 'error');
      return;
    }
    
    setValidationStatus('validating');
    try {
      const isValid: any = await invoke('validate_hubcap_key', { apiKey });
      if (isValid) {
        setValidationStatus('valid');
        showStatus('Chiave API di Hubcap valida e connessa con successo!', 'success');
      } else {
        setValidationStatus('invalid');
        showStatus('Chiave API di Hubcap non valida o scaduta.', 'error');
      }
    } catch (err: any) {
      setValidationStatus('invalid');
      showStatus(`Errore durante la convalida della chiave: ${err}`, 'error');
    }
  };

  return (
    <div className="settings-view">
      <div className="settings-header">
        <h1 className="settings-title">Impostazioni</h1>
        <p className="settings-subtitle">Gestisci le configurazioni di sistema, la chiave API di Hubcap e i percorsi di iniezione di Steam.</p>
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
          <label className="settings-label">Chiave API Hubcap</label>
          <p className="settings-desc">Inserisci la tua chiave API di hubcapmanifest.com per sbloccare la consultazione del database e i download.</p>
          <div className="api-input-row">
            <input 
              type="password" 
              placeholder="Inserisci la chiave API (es. smm_...)"
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
              {validationStatus === 'validating' ? 'Verifica...' :
               validationStatus === 'valid' ? 'Connesso ✓' :
               validationStatus === 'invalid' ? 'Non Valida ✗' : 'Verifica Chiave'}
            </button>
          </div>
        </div>

        {/* Steam Path Section */}
        <div className="settings-group">
          <label className="settings-label">Percorso di Installazione Steam</label>
          <p className="settings-desc">Il percorso della directory principale in cui è installato Steam sul tuo PC (necessario per l'installazione delle DLL e configurazioni).</p>
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
          <label className="settings-label">Libreria Steam Attiva (Opzionale)</label>
          <p className="settings-desc">Se i tuoi giochi sono installati in una libreria secondaria (es. su un hard disk secondario D:\), specifica qui il percorso della cartella principale (es. D:\SteamLibrary).</p>
          <input 
            type="text" 
            placeholder="Lascia vuoto per utilizzare la libreria di default di Steam"
            value={activeLibrary}
            onChange={(e) => setActiveLibrary(e.target.value)}
            className="settings-input"
          />
        </div>

        <div className="settings-separator"></div>

        <div className="form-actions">
          <button type="submit" className="save-settings-btn">
            Salva Impostazioni
          </button>
        </div>
      </form>
    </div>
  );
};
