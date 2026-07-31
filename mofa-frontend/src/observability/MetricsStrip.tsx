import React from 'react';
import { Card } from '../shared/Card';
import { AnimatedNumber } from '../shared/AnimatedNumber';

interface MetricsStripProps {
  status: { memory_used_bytes: number; memory_budget_bytes: number; loaded_models?: number; total_models?: number } | null;
  totalRequestCount: number;
  p50: number;
  p95: number;
  p99: number;
  usdSavedByLocal: string;
}

export function MetricsStrip({ status, totalRequestCount, p50, p95, p99, usdSavedByLocal }: MetricsStripProps) {
  const memoryPercent = status && status.memory_budget_bytes > 0
    ? Number(((status.memory_used_bytes / status.memory_budget_bytes) * 100).toFixed(1))
    : 0;

  return (
    <div className="grid grid-cols-2 md:grid-cols-5 gap-4 mb-6">
      <Card className="p-4 flex flex-col justify-between">
        <div className="text-[11px] font-medium text-text-dim uppercase tracking-wider mb-1">Total Requests</div>
        <div className="text-[24px] font-medium text-text-primary tracking-tight">
          <AnimatedNumber value={totalRequestCount} />
        </div>
      </Card>
      
      <Card className="p-4 flex flex-col justify-between">
        <div className="text-[11px] font-medium text-text-dim uppercase tracking-wider mb-1">Latency Percentiles</div>
        {p50 > 0 ? (
          <div className="flex items-center gap-2 font-mono text-[11px]">
            <span className="text-accent-green">P50: {(p50 / 1000).toFixed(2)}s</span>
            <span className="text-accent-yellow">P95: {(p95 / 1000).toFixed(2)}s</span>
            <span className="text-accent-red">P99: {(p99 / 1000).toFixed(2)}s</span>
          </div>
        ) : (
          <div className="text-[24px] font-medium text-text-primary tracking-tight">--</div>
        )}
      </Card>
      
      <Card className="p-4 flex flex-col justify-between">
        <div className="text-[11px] font-medium text-text-dim uppercase tracking-wider mb-1">Models Loaded</div>
        <div className="text-[24px] font-medium text-text-primary tracking-tight">
          {status?.loaded_models ?? '--'} <span className="text-[14px] font-normal text-text-dim">/ {status?.total_models ?? '--'}</span>
        </div>
      </Card>
      
      <Card className="p-4 flex flex-col justify-between">
        <div className="text-[11px] font-medium text-text-dim uppercase tracking-wider mb-1">Memory Usage</div>
        <div className="text-[24px] font-medium text-text-primary tracking-tight">
          <AnimatedNumber value={memoryPercent} format={(v) => v.toFixed(1)} />
          <span className="text-[14px] font-normal text-text-dim">%</span>
        </div>
      </Card>

      <Card className="p-4 flex flex-col justify-between border-accent-green/20 bg-accent-green/5">
        <div className="text-[11px] font-medium text-accent-green uppercase tracking-wider mb-1">Local Savings</div>
        <div className="text-[24px] font-medium text-accent-green tracking-tight">
          ${usdSavedByLocal}
        </div>
      </Card>
    </div>
  );
}
