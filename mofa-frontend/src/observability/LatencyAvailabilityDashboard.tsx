import React from 'react';
import { Card } from '../shared/Card';
import { AnimatedNumber } from '../shared/AnimatedNumber';
import { Sparkline } from '../shared/Sparkline';
import { DeltaIndicator } from '../shared/DeltaIndicator';
import { Skeleton } from '../shared/Skeleton';

interface LatencyAvailabilityDashboardProps {
  localRequests: number;
  memUsedGb: string;
  memBudgetGb: string;
  memPercent: number;
  totalLocalRequests: number;
  warmupHits: number;
  historyData?: number[];
  lastUpdated?: number;
  prevSessionDelta?: number;
  isLoading?: boolean;
}

export function LatencyAvailabilityDashboard({
  localRequests,
  memUsedGb,
  memBudgetGb,
  memPercent,
  totalLocalRequests,
  warmupHits,
  historyData = [],
  lastUpdated,
  prevSessionDelta = 0,
  isLoading = false
}: LatencyAvailabilityDashboardProps) {
  const latencySavedSec = (warmupHits * 1.45).toFixed(2);
  const secondsAgo = lastUpdated ? Math.floor((Date.now() - lastUpdated) / 1000) : 0;
  const isStale = secondsAgo > 10;

  if (isLoading) {
    return (
      <Card className="p-6 h-full flex flex-col justify-between">
        <div>
          <div className="flex justify-between items-center mb-6">
            <Skeleton className="h-4 w-32" />
            <Skeleton className="h-3 w-20" />
          </div>
          <div className="grid grid-cols-2 gap-6 py-2 border-b border-border-strong pb-6">
            <div className="space-y-2">
              <Skeleton className="h-3 w-24" />
              <Skeleton className="h-8 w-16" />
              <Skeleton className="h-3 w-20" />
            </div>
            <div className="space-y-2">
              <Skeleton className="h-3 w-24" />
              <Skeleton className="h-8 w-20" />
              <Skeleton className="h-1.5 w-full" />
            </div>
          </div>
        </div>
        <div className="space-y-3 pt-5">
          <Skeleton className="h-3 w-full" />
          <Skeleton className="h-3 w-full" />
          <Skeleton className="h-3 w-full" />
        </div>
      </Card>
    );
  }

  return (
    <Card className={`p-6 h-full flex flex-col justify-between transition-opacity duration-300 ${isStale ? 'opacity-70' : 'opacity-100'}`}>
      <div>
        {/* Header */}
        <div className="flex items-center justify-between mb-6">
          <div className="flex items-center gap-2">
            <span className="w-1.5 h-1.5 rounded-full bg-accent-green" />
            <h3 className="text-[12px] font-semibold uppercase tracking-wider text-text-dim">
              Local Hardware Track
            </h3>
          </div>
          <div className="flex items-center gap-2 font-mono text-[11px] text-text-dim">
            <span>locality="local"</span>
            {lastUpdated && (
              <span className={`text-[10px] ${isStale ? 'text-accent-red font-medium' : 'text-text-dim'}`}>
                ({secondsAgo}s ago)
              </span>
            )}
          </div>
        </div>

        {/* Flat Grid Metrics */}
        <div className="grid grid-cols-2 gap-6 py-2 border-b border-border-subtle pb-6">
          <div className="pr-4 border-r border-border-subtle">
            <div className="flex items-center justify-between">
              <span className="text-[12px] text-text-secondary">Session Inferences</span>
              {historyData.length > 1 && (
                <Sparkline data={historyData} color="#22c55e" width={48} height={14} />
              )}
            </div>
            <div className="text-[32px] font-medium tracking-tight text-text-primary mt-1">
              <AnimatedNumber value={localRequests} />
            </div>
            <div className="text-[11px] text-text-dim font-mono mt-1">
              zero cloud egress
            </div>
          </div>

          <div className="pl-2">
            <div className="flex items-center justify-between">
              <span className="text-[12px] text-text-secondary">RAM / VRAM</span>
              <span className="text-[11px] font-mono text-text-dim">{memPercent}%</span>
            </div>
            <div className="text-[32px] font-medium tracking-tight text-text-primary mt-1">
              {memUsedGb} <span className="text-[14px] font-normal text-text-dim">GB</span>
            </div>
            <div className="w-full bg-white/10 rounded-full h-1 mt-3 overflow-hidden">
              <div 
                className={`h-full transition-all duration-300 ${
                  memPercent > 85 ? 'bg-accent-red' : 'bg-text-primary'
                }`}
                style={{ width: `${Math.max(2, memPercent)}%` }}
              />
            </div>
          </div>
        </div>
      </div>

      {/* Footer Metrics */}
      <div className="space-y-2.5 text-[12px] text-text-secondary pt-5">
        <div className="flex justify-between items-center">
          <span>All-Time Workloads</span>
          <div className="flex items-center gap-1.5 font-mono text-text-primary">
            <AnimatedNumber value={totalLocalRequests} />
            <span>requests</span>
            <DeltaIndicator value={prevSessionDelta} />
          </div>
        </div>
        <div className="flex justify-between items-center">
          <span>Cold-Starts Avoided</span>
          <span className="font-mono text-text-primary">
            <AnimatedNumber value={warmupHits} /> {warmupHits > 0 ? `(~${latencySavedSec}s saved)` : ''}
          </span>
        </div>
        <div className="flex justify-between items-center">
          <span>Memory Budget</span>
          <span className="font-mono text-text-primary">{memUsedGb} / {memBudgetGb} GB</span>
        </div>
      </div>
    </Card>
  );
}
