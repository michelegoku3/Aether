import { useState } from 'react';
import { Sidebar, TabType } from './components/Sidebar';
import { MainContent } from './components/MainContent';

export default function App() {
  // Setup state to manage the active view, defaulting to 'home'
  const [activeTab, setActiveTab] = useState<TabType>('home');

  return (
    <div className="app-container">
      {/* Modular Sidebar component */}
      <Sidebar activeTab={activeTab} onTabChange={setActiveTab} />

      {/* Modular Main Content display area */}
      <MainContent activeTab={activeTab} />
    </div>
  );
}
