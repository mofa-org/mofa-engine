import React, { useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Card } from '../shared/Card';
import { Button } from '../shared/Button';
import { X, Volume2, VolumeX, Eye, EyeOff, Trash2 } from 'lucide-react';
import { RealEngineClient } from '../engine/client';
import { getStoredEngineUrl, setStoredEngineUrl } from '../engine/index';
import { useQueryClient } from '@tanstack/react-query';
import { useEngineConnection } from '../engine/useEngineConnection';
import { useSettings } from '../storage/useSettings';
import { useHistory } from '../storage/useHistory';

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export function SettingsModal({ isOpen, onClose }: SettingsModalProps) {
  const [urlInput, setUrlInput] = useState(getStoredEngineUrl());
  const [testResult, setTestResult] = useState<{ success: boolean; text: string } | null>(null);
  const queryClient = useQueryClient();
  const { uptime_secs, version } = useEngineConnection();
  const { settings, updateSettings } = useSettings();
  const { clearHistory } = useHistory();

  const handleSave = () => {
    setStoredEngineUrl(urlInput);
    // Invalidate queries to restart polling with new URL
    queryClient.invalidateQueries({ queryKey: ['engine_health'] });
    queryClient.invalidateQueries({ queryKey: ['engine_status'] });
    onClose();
  };

  const handleTest = async () => {
    setTestResult(null);
    try {
      const tempClient = new RealEngineClient(urlInput);
      const res = await tempClient.getHealth();
      if (res.success) {
        setTestResult({ success: true, text: `Success! Engine v${res.data.version}` });
      } else {
        setTestResult({ success: false, text: res.error || 'Connection failed' });
      }
    } catch {
      setTestResult({ success: false, text: 'Connection failed' });
    }
  };

  const handleClearData = () => {
    if (confirm('Are you sure you want to delete all history and drafts? This cannot be undone.')) {
      clearHistory();
      localStorage.removeItem('mofa_compose_draft');
      alert('Data cleared successfully.');
    }
  };

  return (
    <AnimatePresence>
      {isOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm">
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-label="Settings"
            initial={{ opacity: 0, scale: 0.95, y: 10 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.95, y: 10 }}
            transition={{ duration: 0.3, ease: [0.25, 0.1, 0.25, 1] }}
            className="w-full max-w-md max-h-[90vh] flex flex-col"
          >
            <Card className="flex flex-col overflow-hidden shadow-xl">
              <div className="flex items-center justify-between p-4 border-b border-border-strong shrink-0 bg-background-secondary/50">
                <h2 className="text-lg font-medium text-text-primary">Settings</h2>
                <button onClick={onClose} className="p-1 text-text-secondary hover:text-text-primary transition-colors">
                  <X size={20} />
                </button>
              </div>
              
              <div className="p-4 space-y-6 overflow-y-auto">
                <div className="space-y-4">
                  <div className="space-y-2">
                    <label className="text-sm font-medium text-text-primary">Engine URL</label>
                    <div className="flex gap-2">
                      <input
                        type="text"
                        value={urlInput}
                        onChange={(e) => setUrlInput(e.target.value)}
                        className="flex-1 bg-background-secondary border border-border-strong rounded-[var(--radius-small)] px-3 py-2 text-sm text-text-primary focus:outline-none focus:border-accent-blue"
                        placeholder="http://127.0.0.1:8420"
                      />
                    </div>
                    <div className="flex items-center justify-between mt-2">
                      <Button variant="secondary" onClick={handleTest} className="h-8 px-3 text-xs">
                        Test Connection
                      </Button>
                      {testResult && (
                        <span className={`text-xs ${testResult.success ? 'text-accent-green' : 'text-accent-red'}`}>
                          {testResult.text}
                        </span>
                      )}
                    </div>
                  </div>

                  <div className="space-y-2 pt-4 border-t border-border-strong">
                    <label className="text-sm font-medium text-text-primary">Default Voice</label>
                    <select
                      value={settings.defaultVoice}
                      onChange={(e) => updateSettings({ defaultVoice: e.target.value })}
                      className="w-full bg-background-secondary border border-border-strong rounded-[var(--radius-small)] px-3 py-2 text-sm text-text-primary focus:outline-none focus:border-accent-purple appearance-none"
                    >
                      {['Xiaoxiao', 'Yunxi', 'Xiaoni', 'Nova', 'Alloy', 'Echo'].map((v) => (
                        <option key={v} value={v} className="bg-background-secondary">{v}</option>
                      ))}
                    </select>
                  </div>

                  <div className="pt-4 border-t border-border-strong flex items-center justify-between">
                    <div>
                      <h3 className="text-sm font-medium text-text-primary">Sound Effects</h3>
                      <p className="text-[11px] text-text-dim">Play a chime when generation completes</p>
                    </div>
                    <button
                      onClick={() => updateSettings({ soundEnabled: !settings.soundEnabled })}
                      className={`p-2 rounded-full transition-colors ${settings.soundEnabled ? 'bg-accent-cyan/10 text-accent-cyan' : 'bg-background-hover text-text-dim hover:text-text-secondary'}`}
                    >
                      {settings.soundEnabled ? <Volume2 className="w-5 h-5" /> : <VolumeX className="w-5 h-5" />}
                    </button>
                  </div>

                  <div className="pt-4 border-t border-border-strong flex items-center justify-between">
                    <div>
                      <h3 className="text-sm font-medium text-text-primary">Reduced Motion</h3>
                      <p className="text-[11px] text-text-dim">Minimize animations and transitions</p>
                    </div>
                    <button
                      onClick={() => updateSettings({ reducedMotion: !settings.reducedMotion })}
                      className={`p-2 rounded-full transition-colors ${settings.reducedMotion ? 'bg-accent-blue/10 text-accent-blue' : 'bg-background-hover text-text-dim hover:text-text-secondary'}`}
                    >
                      {settings.reducedMotion ? <EyeOff className="w-5 h-5" /> : <Eye className="w-5 h-5" />}
                    </button>
                  </div>

                  <div className="pt-4 border-t border-border-strong flex items-center justify-between">
                    <div>
                      <h3 className="text-sm font-medium text-accent-red">Danger Zone</h3>
                      <p className="text-[11px] text-text-dim">Clear local drafts and podcast history</p>
                    </div>
                    <Button variant="secondary" onClick={handleClearData} className="text-accent-red border-accent-red/20 hover:bg-accent-red/10 h-8 px-3 text-xs gap-1.5">
                      <Trash2 className="w-3.5 h-3.5" />
                      Clear Data
                    </Button>
                  </div>
                </div>

                <div className="space-y-2 pt-4 border-t border-border-strong">
                  <h3 className="text-sm font-medium text-text-primary">About MoFA FM</h3>
                  <div className="text-xs text-text-secondary space-y-1">
                    <p>Engine Version: {version || 'Offline'}</p>
                    <p>Uptime: {uptime_secs ? `${uptime_secs}s` : 'Offline'}</p>
                    <p>Frontend: v0.1.0</p>
                    <div className="flex gap-3 pt-2">
                      <a href="https://github.com/mofa-org/mofa-engine" target="_blank" rel="noreferrer" className="text-accent-blue hover:underline">GitHub</a>
                      <a href="http://localhost:3000" target="_blank" rel="noreferrer" className="text-accent-blue hover:underline">Grafana Metrics</a>
                    </div>
                  </div>
                </div>
              </div>

              <div className="p-4 border-t border-border-strong flex justify-end gap-2 bg-background-hover shrink-0">
                <Button variant="primary" onClick={handleSave} className="w-full">Done</Button>
              </div>
            </Card>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  );
}
