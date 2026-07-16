import React, { useEffect, useState } from 'react';
import { engine } from '../engine';
import { EngineStatus } from '../engine/types';
import { Card } from '../shared/Card';
import { Activity, Clock, Cpu, HardDrive } from 'lucide-react';

export function MetricsStrip() {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [requestCount, setRequestCount] = useState(0);
  const [avgLatency, setAvgLatency] = useState(0);

  useEffect(() => {
    // Initial fetch
    engine.getStatus().then(res => {
      if (res.success) setStatus(res.data);
    });

    const handleEvent = (evt: any) => {
      if (evt.type === 'RequestStarted') {
        setRequestCount(prev => prev + 1);
      } else if (evt.type === 'RequestCompleted') {
        if (evt.data && evt.data.duration_ms) {
          setAvgLatency(prev => {
            if (prev === 0) return evt.data.duration_ms;
            return prev * 0.8 + evt.data.duration_ms * 0.2;
          });
        }
      } else if (
        evt.type === 'ModelStatusChanged' ||
        evt.type === 'ModelResidencyChanged' ||
        evt.type === 'MemoryChanged'
      ) {
        engine.getStatus().then(res => {
          if (res.success) setStatus(res.data);
        });
      }
    };

    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => unsubscribe();
  }, []);

  const memoryPercent = status 
    ? ((status.memory_used_bytes / status.memory_budget_bytes) * 100).toFixed(1) 
    : 0;

  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">
      <Card className="p-4 flex items-center gap-4">
        <div className="w-10 h-10 rounded-full bg-accent-blue/10 flex items-center justify-center">
          <Activity className="w-5 h-5 text-accent-blue" />
        </div>
        <div>
          <div className="text-[11px] font-medium text-text-secondary uppercase tracking-wider mb-1">Total Requests</div>
          <div className="text-[20px] font-semibold text-text-primary leading-none">{requestCount}</div>
        </div>
      </Card>
      
      <Card className="p-4 flex items-center gap-4">
        <div className="w-10 h-10 rounded-full bg-accent-purple/10 flex items-center justify-center">
          <Clock className="w-5 h-5 text-accent-purple" />
        </div>
        <div>
          <div className="text-[11px] font-medium text-text-secondary uppercase tracking-wider mb-1">Avg Latency</div>
          <div className="text-[20px] font-semibold text-text-primary leading-none">
            {avgLatency > 0 ? `${(avgLatency / 1000).toFixed(2)}s` : '--'}
          </div>
        </div>
      </Card>
      
      <Card className="p-4 flex items-center gap-4">
        <div className="w-10 h-10 rounded-full bg-accent-green/10 flex items-center justify-center">
          <Cpu className="w-5 h-5 text-accent-green" />
        </div>
        <div>
          <div className="text-[11px] font-medium text-text-secondary uppercase tracking-wider mb-1">Models Loaded</div>
          <div className="text-[20px] font-semibold text-text-primary leading-none">
            {status?.loaded_models ?? '--'} <span className="text-sm font-normal text-text-dim">/ {status?.total_models ?? '--'}</span>
          </div>
        </div>
      </Card>
      
      <Card className="p-4 flex items-center gap-4">
        <div className="w-10 h-10 rounded-full bg-accent-yellow/10 flex items-center justify-center">
          <HardDrive className="w-5 h-5 text-accent-yellow" />
        </div>
        <div>
          <div className="text-[11px] font-medium text-text-secondary uppercase tracking-wider mb-1">Memory Usage</div>
          <div className="text-[20px] font-semibold text-text-primary leading-none">
            {memoryPercent}%
          </div>
        </div>
      </Card>
    </div>
  );
}
