import React, { useEffect, useState, useRef } from 'react';
import { engine } from '../engine';

export function MemoryGauge() {
  const [memory, setMemory] = useState<number>(0);
  const [maxMemory, setMaxMemory] = useState<number>(16 * 1024 * 1024 * 1024);
  const [delta, setDelta] = useState<number | null>(null);
  const deltaTimeout = useRef<any>(null);

  useEffect(() => {
    // Initial fetch
    engine.getStatus().then(s => {
      if (s.success) {
        setMemory(s.data.memory_used_bytes || 0);
        setMaxMemory(s.data.memory_budget_bytes || 16 * 1024 * 1024 * 1024);
      }
    });

    const handleEvent = (evt: any) => {
      if (evt.type === 'MemoryChanged') {
        setMemory(prev => {
          const newMem = evt.data.used_bytes;
          const diff = newMem - prev;
          if (Math.abs(diff) > 1024 * 1024) { // only show significant delta
            setDelta(diff);
            if (deltaTimeout.current) clearTimeout(deltaTimeout.current);
            deltaTimeout.current = setTimeout(() => setDelta(null), 2000);
          }
          return newMem;
        });
      }
    };
    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => {
      unsubscribe();
      if (deltaTimeout.current) clearTimeout(deltaTimeout.current);
    };
  }, []);

  const percentage = Math.min(100, Math.max(0, (memory / maxMemory) * 100));
  const memoryGb = (memory / 1024 / 1024 / 1024).toFixed(2);
  const maxGb = (maxMemory / 1024 / 1024 / 1024).toFixed(1);

  let colorClass = 'bg-accent-green';
  let textColorClass = 'text-accent-green';
  if (percentage >= 90) {
    colorClass = 'bg-accent-red';
    textColorClass = 'text-accent-red';
  } else if (percentage >= 80) {
    colorClass = 'bg-accent-yellow';
    textColorClass = 'text-accent-yellow';
  }

  return (
    <div className="w-full">
      <div className="flex justify-between items-end mb-2">
        <div className="flex items-end gap-1">
          <span className="text-xl font-bold font-mono tracking-tight text-text-primary leading-none">{memoryGb}</span>
          <span className="text-xs text-text-dim mb-0.5 leading-none">/ {maxGb} GB</span>
        </div>
        <div className={`text-[11px] font-bold ${textColorClass}`}>
          {percentage.toFixed(1)}%
        </div>
      </div>
      <div className="h-2 w-full bg-background-hover rounded-full overflow-hidden">
        <div 
          className={`h-full ${colorClass} transition-all duration-1000 ease-out`}
          style={{ width: `${percentage}%` }}
        />
      </div>
      <div className="h-4 mt-1 flex justify-end">
        {delta !== null && (
          <span className={`text-[10px] font-mono font-medium animate-fade-in ${delta > 0 ? 'text-accent-red' : 'text-accent-green'}`}>
            {delta > 0 ? '+' : ''}{(delta / 1024 / 1024 / 1024).toFixed(2)} GB
          </span>
        )}
      </div>
    </div>
  );
}
