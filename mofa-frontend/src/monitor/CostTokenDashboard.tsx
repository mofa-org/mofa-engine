import React, { useEffect, useState } from 'react';
import { engine } from '../engine';

export function CostTokenDashboard() {
  const [cacheHits, setCacheHits] = useState(0);
  const [totalTokens, setTotalTokens] = useState(0);

  useEffect(() => {
    const handleEvent = (evt: any) => {
      if (evt.type === 'RequestCompleted') {
        const data = evt.data;
        if (data.tokens_used) {
          setTotalTokens(prev => prev + data.tokens_used);
        }
        if (data.tokens_cache_hit) {
          setCacheHits(prev => prev + data.tokens_cache_hit);
        }
      }
    };
    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => unsubscribe();
  }, []);

  const ratio = totalTokens > 0 ? (cacheHits / totalTokens) * 100 : 0;

  return (
    <div className="flex flex-col gap-2 p-3 bg-background-secondary border border-border-subtle rounded-md shadow-sm">
      <div className="flex justify-between items-center text-xs">
        <span className="text-text-dim font-medium uppercase tracking-wider text-[10px]">Prompt Cache Hit Ratio</span>
        <span className="font-medium text-accent-green">{ratio.toFixed(1)}%</span>
      </div>
      <div className="w-full h-1.5 bg-background-hover rounded-full overflow-hidden mt-1">
        <div className="h-full bg-accent-green transition-all duration-500" style={{ width: `${ratio}%` }} />
      </div>
      <div className="flex justify-between text-[10px] text-text-dim mt-1">
        <span>{cacheHits.toLocaleString()} cached</span>
        <span>{totalTokens.toLocaleString()} total</span>
      </div>
    </div>
  );
}
