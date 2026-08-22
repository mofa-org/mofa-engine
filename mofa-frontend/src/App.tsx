import React, { useState, useEffect } from 'react';
import { Studio } from './studio/Studio';
import { TopBar } from './shell/TopBar';
import { useEngineConnection } from './engine/useEngineConnection';
import { EngineOffline } from './errors/EngineOffline';
import { MonitorSidebar } from './monitor/MonitorSidebar';
import { HistoryDrawer } from './storage/HistoryDrawer';
import { CommandPalette } from './shell/CommandPalette';
import { AnimatePresence, motion, MotionConfig } from 'framer-motion';
import { useSettings } from './storage/useSettings';
import { ObservabilityView } from './observability/ObservabilityView';
import { ArtifactsGallery } from './studio/gallery/ArtifactsGallery';

class ErrorBoundary extends React.Component<{children: React.ReactNode}, {hasError: boolean}> {
  constructor(props: {children: React.ReactNode}) {
    super(props);
    this.state = { hasError: false };
  }
  static getDerivedStateFromError(_error: any) {
    return { hasError: true };
  }
  render() {
    if (this.state.hasError) {
      return (
        <div className="min-h-screen flex items-center justify-center bg-background-primary text-text-primary p-6">
          <div className="bg-background-secondary border border-border-strong rounded-xl p-8 max-w-md w-full text-center">
            <h2 className="text-xl font-bold mb-4">Something went wrong</h2>
            <p className="text-text-secondary mb-6 text-sm">An unexpected error occurred in the application.</p>
            <button 
              onClick={() => window.location.reload()} 
              className="px-4 py-2 bg-accent-cyan text-white rounded font-medium text-sm hover:bg-accent-cyan/90 transition-colors"
            >
              Reload Application
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

export default function App() {
  const { state } = useEngineConnection();
  const { settings } = useSettings();
  const [currentView, setCurrentView] = useState<'studio' | 'observability' | 'artifacts'>('studio');

  useEffect(() => {
    const handleNavigate = (e: any) => setCurrentView(e.detail);
    document.addEventListener('navigate', handleNavigate);
    return () => document.removeEventListener('navigate', handleNavigate);
  }, []);

  return (
    <ErrorBoundary>
      <MotionConfig reducedMotion={settings.reducedMotion ? "always" : "user"}>
        <motion.div 
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.4, ease: [0.25, 0.1, 0.25, 1] }}
          className="h-screen w-full flex flex-col bg-background-primary text-text-primary relative overflow-hidden"
        >
          <TopBar currentView={currentView} />
          {currentView === 'studio' && (
            <div className="flex-1 flex overflow-hidden max-w-[1400px] mx-auto w-full">
              <Studio />
              <MonitorSidebar />
            </div>
          )}
          {currentView === 'observability' && (
            <ObservabilityView />
          )}
          {currentView === 'artifacts' && (
            <div className="flex-1 flex overflow-hidden max-w-[1400px] mx-auto w-full">
              <ArtifactsGallery />
            </div>
          )}
          <HistoryDrawer />
          <CommandPalette />
          
          <AnimatePresence>
            {state === 'disconnected' && <EngineOffline />}
          </AnimatePresence>
        </motion.div>
      </MotionConfig>
    </ErrorBoundary>
  );
}
