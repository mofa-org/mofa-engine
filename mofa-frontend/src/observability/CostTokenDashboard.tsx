import React, { useEffect, useState } from 'react';
import { Card } from '../shared/Card';
import { AnimatedNumber } from '../shared/AnimatedNumber';
import { Sparkline } from '../shared/Sparkline';
import { DeltaIndicator } from '../shared/DeltaIndicator';
import { Skeleton } from '../shared/Skeleton';

interface CostTokenDashboardProps {
  cloudRequests: number;
  cloudCostUsd: number;
  totalCloudRequests: number;
  totalCloudCost: number;
  quotaErrors: number;
  budgetCapUsd?: number;
  historyData?: number[];
  lastUpdated?: number;
  prevSessionDelta?: number;
  isLoading?: boolean;
}

export function CostTokenDashboard({
  cloudRequests,
  cloudCostUsd,
  totalCloudRequests,
  totalCloudCost,
  quotaErrors,
  budgetCapUsd = 1.00,
  historyData = [],
  lastUpdated,
  prevSessionDelta = 0,
  isLoading = false
}: CostTokenDashboardProps) {
  const [sessionStartTime] = useState(new Date());
  const [projectedHourlyCost, setProjectedHourlyCost] = useState(0);

  useEffect(() => {
    const interval = setInterval(() => {
      const now = new Date();
      const sessionDurationMs = Math.max(1, now.getTime() - sessionStartTime.getTime());
      const hours = sessionDurationMs / (1000 * 60 * 60);
      if (hours > 0) {
        setProjectedHourlyCost(cloudCostUsd / hours);
      }
    }, 3000);
    
    return () => clearInterval(interval);
  }, [cloudCostUsd, sessionStartTime]);

  const estimatedPromptTokens = cloudRequests * 180;
  const estimatedCompletionTokens = cloudRequests * 320;
  const totalSessionTokens = estimatedPromptTokens + estimatedCompletionTokens;
  const promptPercent = totalSessionTokens > 0 ? Math.round((estimatedPromptTokens / totalSessionTokens) * 100) : 36;
  const completionPercent = 100 - promptPercent;
  const budgetPercent = Math.min(100, Math.round((totalCloudCost / budgetCapUsd) * 100));

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
              <Skeleton className="h-3 w-24" />
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
            <span className="w-1.5 h-1.5 rounded-full bg-orange-500" />
            <h3 className="text-[12px] font-semibold uppercase tracking-wider text-text-dim">
              Cloud Financial Track
            </h3>
          </div>
          <div className="flex items-center gap-2 font-mono text-[11px] text-text-dim">
            <span>locality="cloud"</span>
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
            <span className="text-[12px] text-text-secondary">Session Inferences</span>
            <div className="text-[32px] font-medium tracking-tight text-text-primary mt-1">
              <AnimatedNumber value={cloudRequests} />
            </div>
            <div className="text-[11px] text-text-dim font-mono mt-1">
              ~{totalSessionTokens.toLocaleString()} tokens
            </div>
          </div>

          <div className="pl-2">
            <div className="flex items-center justify-between">
              <span className="text-[12px] text-text-secondary">Session Spend</span>
              {historyData.length > 1 && (
                <Sparkline data={historyData} color="#f97316" width={48} height={14} />
              )}
            </div>
            <div className="text-[32px] font-medium tracking-tight text-text-primary mt-1">
              $<AnimatedNumber value={cloudCostUsd} format={(v) => v.toFixed(5)} />
            </div>
            <div className="text-[11px] text-text-dim font-mono mt-1">
              {projectedHourlyCost > 0 ? `est. $${projectedHourlyCost.toFixed(4)}/hr` : 'pay-per-token'}
            </div>
          </div>
        </div>

        {/* Token Volume Split - Minimal Line */}
        <div className="py-4 border-b border-border-subtle">
          <div className="flex items-center justify-between text-[11px] text-text-dim font-mono mb-2">
            <span>Token Split</span>
            <span>In: {estimatedPromptTokens} / Out: {estimatedCompletionTokens}</span>
          </div>
          <div className="w-full bg-white/10 rounded-full h-1 flex overflow-hidden">
            <div className="bg-text-secondary h-full" style={{ width: `${promptPercent}%` }} title="Prompt Tokens" />
            <div className="bg-text-dim h-full" style={{ width: `${completionPercent}%` }} title="Completion Tokens" />
          </div>
        </div>

        {/* Budget Cap Usage Gauge */}
        <div className="py-3 border-b border-border-subtle">
          <div className="flex items-center justify-between text-[11px] text-text-dim font-mono mb-1.5">
            <span>Budget Cap Usage</span>
            <span>{budgetPercent}% of ${budgetCapUsd.toFixed(2)}</span>
          </div>
          <div className="w-full bg-white/10 rounded-full h-1 overflow-hidden">
            <div 
              className={`h-full transition-all duration-300 ${
                budgetPercent > 85 ? 'bg-accent-red' : budgetPercent > 60 ? 'bg-accent-yellow' : 'bg-text-secondary'
              }`}
              style={{ width: `${Math.max(2, budgetPercent)}%` }}
            />
          </div>
        </div>
      </div>

      {/* Footer Metrics */}
      <div className="space-y-2.5 text-[12px] text-text-secondary pt-4">
        <div className="flex justify-between items-center">
          <span>All-Time Spend</span>
          <div className="flex items-center gap-1.5 font-mono text-text-primary">
            <span>${totalCloudCost.toFixed(5)}</span>
            <span className="text-text-dim">({totalCloudRequests} reqs)</span>
            <DeltaIndicator value={prevSessionDelta} />
          </div>
        </div>
        <div className="flex justify-between items-center">
          <span>Rate Limits (429)</span>
          {quotaErrors > 0 ? (
            <span className="font-mono text-accent-red flex items-center gap-1.5">
              <span className="w-1.5 h-1.5 rounded-full bg-accent-red" />
              {quotaErrors} errors
            </span>
          ) : (
            <span className="font-mono text-text-primary">0 errors</span>
          )}
        </div>
        <div className="flex justify-between items-center">
          <span>Active Provider</span>
          <span className="font-mono text-text-primary">Cloud Provider</span>
        </div>
      </div>
    </Card>
  );
}
