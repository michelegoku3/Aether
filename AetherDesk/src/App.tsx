import { useState } from 'react';
import { Sidebar, TabType } from './components/Sidebar';
import { MainContent } from './components/MainContent';
import { invoke } from '@tauri-apps/api/core';

export default function App() {
  // Setup state to manage the active view, defaulting to 'home'
  const [activeTab, setActiveTab] = useState<TabType>('home');

  // Professional decoupled action handler to kill and restart Steam via Rust
  const handleRestartSteam = async () => {
    try {
      await invoke('restart_steam');
      alert('Steam has been terminated and is restarting asynchronously in the background.');
    } catch (err: any) {
      alert(`Failed to restart Steam: ${err}`);
    }
  };

  return (
    <div className="app-container">
      {/* Modular Sidebar component with action hooks */}
      <Sidebar 
        activeTab={activeTab} 
        onTabChange={setActiveTab} 
        onRestartSteam={handleRestartSteam} 
      />

      {/* Modular Main Content display area */}
      <MainContent activeTab={activeTab} />
    </div>
  );
}
