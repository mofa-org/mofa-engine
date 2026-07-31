import React, { useEffect, useState } from 'react';
import { engine } from '../engine';
import { ModelCard } from '../engine/types';
import { StatusDot } from '../shared/StatusDot';
import { Cpu } from 'lucide-react';


export function ModelGrid() {
  const [models, setModels] = useState<ModelCard[]>([]);

  useEffect(() => {
    // Initial load
    engine.getCapabilities().then(c => {
      if (c.success) setModels(c.data);
    });

    const handleEvent = (evt: any) => {
      if (evt.type === 'ModelStatusChanged' || evt.type === 'ModelResidencyChanged') {
        engine.getCapabilities().then(c => {
          if (c.success) setModels(c.data);
        });
      }
    };

    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => unsubscribe();
  }, []);

  const activeModels = models.filter(m => (m.status || '').toLowerCase() !== 'cold' || (m.residency || '').toLowerCase() === 'remote');

  if (activeModels.length === 0) {
    return (
      <div className="p-4 border border-border-subtle border-dashed rounded-md flex flex-col items-center justify-center text-center gap-2">
        <Cpu className="w-5 h-5 text-text-dim" />
        <span className="text-[12px] text-text-dim">All models cold on disk</span>
      </div>
    );
  }

  return (
    <div className="grid gap-1">
      {activeModels.map(model => {
        const memGb = model.memory_estimate_bytes ? (model.memory_estimate_bytes / 1024 / 1024 / 1024).toFixed(1) + 'GB' : '0GB';
        const isRemote = (model.residency || '').toLowerCase() === 'remote';
        const statusLower = (model.status || '').toLowerCase();
        
        return (
          <div key={model.id} className={`px-2.5 py-2 border-b border-border-subtle last:border-0 hover:bg-background-hover transition-colors rounded-sm flex items-center justify-between ${isRemote ? 'opacity-70' : ''}`}>
            <div className="flex items-center gap-2.5">
              <StatusDot status={
                isRemote ? 'Healthy' : 
                statusLower === 'hot' || statusLower === 'busy' ? 'Healthy' : 
                statusLower === 'warming' ? 'Connecting' : 
                statusLower === 'failed' ? 'Failed' : 'Failed'
              } />
              <div>
                <div className="text-[12px] font-mono text-text-primary leading-none mb-1 tracking-tight">{model.id}</div>
                <div className="text-[10px] text-text-dim leading-none flex gap-1.5 items-center">
                  <span>{model.provider}</span>
                  {!isRemote && (
                    <>
                      <span>·</span>
                      <span>{memGb}</span>
                    </>
                  )}
                  <span>·</span>
                  <span>
                    {(() => {
                      const t = (model.cost_tier || '').toLowerCase();
                      if (t === 'high') return '~$1.00/1k';
                      if (t === 'medium') return '~$0.50/1k';
                      if (t === 'low') return '~$0.10/1k';
                      return '$0/1k';
                    })()}
                  </span>
                </div>
              </div>
            </div>
            <div className="flex flex-col items-end gap-1">
              <span className={`text-[9px] font-mono px-1 py-0.5 rounded ${
                isRemote ? 'bg-background-hover text-text-secondary' : 
                statusLower === 'hot' ? 'bg-accent-green/10 text-accent-green' : 
                statusLower === 'warming' ? 'bg-accent-cyan/10 text-accent-cyan' :
                'bg-background-hover text-text-dim'
              }`}>
                {isRemote ? 'REMOTE' : statusLower.toUpperCase()}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );

}
