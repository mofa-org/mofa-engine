import React, { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { engine } from '../engine';
import { EngineEvent } from '../engine/types';

interface TelemetryItem {
  id: string;
  type: string;
  model: string;
  backend: string;
  isLocal: boolean;
  isFallback: boolean;
  reason: string;
  timestamp: number;
}

export function TelemetryTicker() {
  const [latestItem, setLatestItem] = useState<TelemetryItem | null>(null);

  useEffect(() => {
    const handleEvent = (evt: EngineEvent) => {
      try {
        if (evt?.type === 'RoutingDecision') {
          const data = evt?.data || {};
          const backend = (data.selected_backend || 'local').toLowerCase();
          const isLocal = backend === 'ollama' || backend === 'kokoro' || backend === 'local';
          
          setLatestItem({
            id: `${Date.now()}-${Math.random()}`,
            type: 'RoutingDecision',
            model: data.selected_model || 'unknown',
            backend: data.selected_backend || 'local',
            isLocal,
            isFallback: !!data.is_fallback,
            reason: data.reason || 'capability_match',
            timestamp: Date.now()
          });
        } else if (evt?.type === 'FailoverTriggered') {
          const data = evt?.data || {};
          setLatestItem({
            id: `${Date.now()}-${Math.random()}`,
            type: 'FailoverTriggered',
            model: data.fallback_model || 'fallback',
            backend: data.fallback_backend || 'local',
            isLocal: true,
            isFallback: true,
            reason: `Failed ${data.failed_backend || 'primary'} -> ${data.fallback_backend || 'fallback'}`,
            timestamp: Date.now()
          });
        }
      } catch (err) {
        console.warn('Failed to parse TelemetryTicker event', err);
      }
    };

    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => unsubscribe();
  }, []);

  return (
    <div className="bg-background-card border border-border-subtle rounded-[var(--radius-card)] px-4 py-2.5 flex items-center justify-between text-[12px] shadow-md">
      <div className="flex items-center gap-2 text-text-secondary">
        <span className="w-2 h-2 rounded-full bg-accent-green" />
        <span className="font-medium text-text-primary text-[12px]">Router Telemetry</span>
      </div>

      <div className="flex-1 max-w-xl mx-4 overflow-hidden h-5 relative flex items-center justify-end">
        <AnimatePresence mode="wait">
          {latestItem ? (
            <motion.div
              key={latestItem.id}
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -6 }}
              transition={{ duration: 0.15 }}
              className="flex items-center gap-2 text-right"
            >
              <span className="font-mono text-text-dim text-[11px]">
                {new Date(latestItem.timestamp).toLocaleTimeString()}
              </span>
              <span className="font-medium text-text-primary text-[12px]">
                {latestItem.model.split('/').pop()}
              </span>
              <span className="px-1.5 py-0.5 rounded text-[10px] font-mono text-text-dim border border-border-subtle bg-background-hover">
                {latestItem.isLocal ? 'local' : 'cloud'}
              </span>
              {latestItem.isFallback && (
                <span className="px-1.5 py-0.5 rounded text-[10px] font-mono text-accent-red border border-accent-red/20 bg-accent-red/5">
                  failover
                </span>
              )}
              <span className="text-text-dim text-[11px] font-mono max-w-[200px] truncate">
                ({latestItem.reason})
              </span>
            </motion.div>
          ) : (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              className="text-text-dim text-[11px] font-mono"
            >
              router active — listening for inference calls...
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      <div className="flex items-center gap-2 text-[11px] text-text-dim font-mono border-l border-border-subtle pl-3">
        <span className="w-1.5 h-1.5 rounded-full bg-accent-green" />
        <span>Connected</span>
      </div>
    </div>
  );
}
