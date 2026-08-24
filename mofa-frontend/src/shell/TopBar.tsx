import React, { useState, useEffect } from 'react';
import { Settings, History, Activity, FolderOpen } from 'lucide-react';
import { SettingsModal } from './SettingsModal';

interface TopBarProps {
  currentView?: 'studio' | 'observability' | 'artifacts';
}

export function TopBar({ currentView = 'studio' }: TopBarProps) {
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    const handleOpen = () => setSettingsOpen(true);
    document.addEventListener('open-settings', handleOpen);
    return () => document.removeEventListener('open-settings', handleOpen);
  }, []);

  return (
    <>
      <header className="h-16 border-b border-border-subtle flex items-center justify-between px-6 bg-background-primary/80 backdrop-blur-md sticky top-0 z-30 shrink-0">
        <div className="flex items-center gap-3 cursor-pointer" onClick={() => document.dispatchEvent(new CustomEvent('navigate', { detail: 'studio' }))}>
          <img src="/mofa-logo.png" alt="MoFA Logo" className="w-8 h-8 rounded-lg shadow-md" />
          <span className="font-medium text-lg tracking-tight text-text-primary">MoFA Engine</span>
        </div>
        
        <div className="flex items-center gap-3">
          <div className="hidden sm:flex items-center gap-1.5 px-2 py-1 rounded bg-background-hover border border-border-subtle text-[11px] font-mono text-text-dim cursor-pointer hover:text-text-secondary transition-colors" onClick={() => document.dispatchEvent(new KeyboardEvent('keydown', { metaKey: true, key: 'k' }))}>
            <span>Cmd+</span>
            <span>K</span>
          </div>
          <button
            onClick={() => document.dispatchEvent(new CustomEvent('navigate', { detail: currentView === 'artifacts' ? 'studio' : 'artifacts' }))}
            className={`flex items-center gap-2 px-3 py-1.5 rounded-full border text-xs transition-colors ${currentView === 'artifacts' ? 'bg-white/5 border-border-strong text-text-primary' : 'bg-background-hover border-border-subtle text-text-secondary hover:bg-white/5 hover:text-text-primary'}`}
          >
            <FolderOpen className="w-3.5 h-3.5" />
            <span>Artifacts</span>
          </button>
          <button
            onClick={() => document.dispatchEvent(new CustomEvent('navigate', { detail: currentView === 'observability' ? 'studio' : 'observability' }))}
            className={`flex items-center gap-2 px-3 py-1.5 rounded-full border text-xs transition-colors ${currentView === 'observability' ? 'bg-white/5 border-border-strong text-text-primary' : 'bg-background-hover border-border-subtle text-text-secondary hover:bg-white/5 hover:text-text-primary'}`}
          >
            <Activity className="w-3.5 h-3.5" />
            <span>Observability</span>
          </button>
          <button
            onClick={() => document.dispatchEvent(new CustomEvent('open-history'))}
            className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-background-hover border border-border-subtle text-xs text-text-secondary hover:bg-white/5 hover:text-text-primary transition-colors"
          >
            <History className="w-3.5 h-3.5" />
            <span>History</span>
          </button>
          <div className="w-px h-4 bg-white/5" />
          <button 
            onClick={() => setSettingsOpen(true)}
            className="p-2 text-text-secondary hover:text-text-primary transition-colors rounded-full hover:bg-background-hover"
          >
            <Settings className="w-5 h-5" />
          </button>
        </div>
      </header>
      <SettingsModal isOpen={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </>
  );
}
