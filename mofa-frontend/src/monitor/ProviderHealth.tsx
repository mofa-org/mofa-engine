import React from 'react';
import { ShieldAlert, ShieldCheck } from 'lucide-react';
import { useEngineStatus } from '../engine/useEngineStatus';

export function ProviderHealth() {
  const { data: statusRes } = useEngineStatus();
  
  const providers = (statusRes?.success ? statusRes.data.provider_health : null) || [
    { name: 'Ollama (Local)', healthy: true, circuit_state: 'closed' },
    { name: 'Kokoro (Local)', healthy: true, circuit_state: 'closed' }
  ];

  return (
    <div className="grid gap-1">
      {providers.map(p => {
        const state = (p.circuit_state || '').toLowerCase();
        const isClosed = state === 'closed';
        const isHalfOpen = state === 'halfopen' || state === 'half_open';
        
        return (
          <div key={p.name} className="px-2.5 py-1.5 border-b border-border-subtle last:border-0 flex items-center justify-between hover:bg-background-hover transition-colors rounded-sm">
            <div className="flex items-center gap-2">
              {isClosed ? (
                <ShieldCheck className="w-3.5 h-3.5 text-accent-green" />
              ) : isHalfOpen ? (
                <ShieldAlert className="w-3.5 h-3.5 text-accent-yellow animate-pulse" />
              ) : (
                <ShieldAlert className="w-3.5 h-3.5 text-accent-red animate-pulse" />
              )}
              <span className="text-[12px] font-medium text-text-primary capitalize">{p.name}</span>
            </div>
            <div className="flex items-center gap-2">
              <span className={`text-[9px] font-mono px-1 py-0.5 rounded ${isClosed ? 'bg-accent-green/10 text-accent-green' : isHalfOpen ? 'bg-accent-yellow/10 text-accent-yellow' : 'bg-accent-red/10 text-accent-red'}`}>
                {isClosed ? 'HEALTHY' : isHalfOpen ? 'HALF_OPEN' : 'OPEN'}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
