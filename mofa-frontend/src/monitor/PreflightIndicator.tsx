import React, { useEffect, useState } from 'react';
import { engine } from '../engine';
import { Zap, RotateCw, CheckCircle2 } from 'lucide-react';

export function PreflightIndicator() {
  const [status, setStatus] = useState<{ type: 'idle' | 'warming' | 'evicting'; model?: string; loadedCount: number }>({
    type: 'idle',
    loadedCount: 0
  });

  useEffect(() => {
    // Initial fetch to get loaded count
    engine.getStatus().then(s => {
      if (s.success) {
        setStatus(prev => ({ ...prev, loadedCount: s.data.loaded_models }));
      }
    });

    const handleEvent = (evt: any) => {
      if (evt.type === 'ModelResidencyChanged') {
        const { model, from: _from, to, reason } = evt.data;
        if (to === 'Loading') {
          setStatus(prev => ({ ...prev, type: 'warming', model }));
        } else if (to === 'Unloaded' && reason === 'eviction') {
          setStatus(prev => ({ ...prev, type: 'evicting', model }));
        } else if (to === 'Loaded' || to === 'Unloaded') {
          // Re-fetch loaded count
          engine.getStatus().then(s => {
            if (s.success) {
              setStatus({ type: 'idle', loadedCount: s.data.loaded_models });
            }
          });
        }
      } else if (evt.type === 'ModelStatusChanged') {
        const { model, from: _from, to } = evt.data;
        if (to === 'Warming') {
          setStatus(prev => ({ ...prev, type: 'warming', model }));
        }
      }
    };

    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => unsubscribe();
  }, []);

  return (
    <div className="p-3 bg-background-hover border border-border-subtle rounded-md flex items-center justify-center min-h-[44px]">
      {status.type === 'idle' ? (
        <div className="flex items-center gap-2 text-text-dim text-[11px] uppercase tracking-wider font-medium">
          <CheckCircle2 className="w-3.5 h-3.5" />
          <span>Idle · {status.loadedCount} Loaded</span>
        </div>
      ) : status.type === 'warming' ? (
        <div className="flex items-center gap-2 text-accent-cyan text-[11px] uppercase tracking-wider font-medium animate-pulse">
          <Zap className="w-3.5 h-3.5" />
          <span>Pre-warming {status.model}</span>
        </div>
      ) : (
        <div className="flex items-center gap-2 text-accent-yellow text-[11px] uppercase tracking-wider font-medium animate-pulse">
          <RotateCw className="w-3.5 h-3.5 animate-spin" />
          <span>Evicting {status.model}</span>
        </div>
      )}
    </div>
  );
}
