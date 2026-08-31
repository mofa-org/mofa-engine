import React, { useEffect, useState } from 'react';
import { useEngineMetrics } from './useEngineMetrics';
import { CostTokenDashboard } from './CostTokenDashboard';
import { LatencyAvailabilityDashboard } from './LatencyAvailabilityDashboard';
import { TelemetryTicker } from './TelemetryTicker';
import { SessionCharts } from './SessionCharts';
import { ActivityFeed } from './ActivityFeed';
import { ModelEfficiencyTable } from './ModelEfficiencyTable';
import { Download } from 'lucide-react';

export function DualTrackView() {
  const {
    status,
    capabilities,
    localRequests,
    cloudRequests,
    cloudCostUsd,
    warmupHits,
    isLoading,
    lastUpdated,
    localHistory,
    velocityHistory,
    costHistory,
    memHistory,
    totalLocalRequests,
    totalCloudRequests,
    totalCloudCost,
    usdSavedByLocal,
    activityFeed,
    refresh
  } = useEngineMetrics();

  const [sessionStartTime] = useState<number>(() => Date.now());
  const [sessionDurationStr, setSessionDurationStr] = useState('0m 0s');
  const [exported, setExported] = useState(false);

  // Session duration timer
  useEffect(() => {
    const timer = setInterval(() => {
      const elapsedSec = Math.floor((Date.now() - sessionStartTime) / 1000);
      const mins = Math.floor(elapsedSec / 60);
      const secs = elapsedSec % 60;
      setSessionDurationStr(`${mins}m ${secs}s`);
    }, 1000);
    return () => clearInterval(timer);
  }, [sessionStartTime]);

  const memUsedGb = status ? (status.memory_used_bytes / (1024 * 1024 * 1024)).toFixed(2) : '0.00';
  const memBudgetGb = status ? (status.memory_budget_bytes / (1024 * 1024 * 1024)).toFixed(1) : '8.0';
  const memPercent = status && status.memory_budget_bytes > 0 
    ? Math.min(100, Math.round((status.memory_used_bytes / status.memory_budget_bytes) * 100))
    : 0;

  const totalSessionReqs = localRequests + cloudRequests;
  const localRatio = totalSessionReqs > 0 ? Math.round((localRequests / totalSessionReqs) * 100) : 100;
  const cloudRatio = totalSessionReqs > 0 ? 100 - localRatio : 0;

  const handleExportTelemetry = () => {
    const payload = {
      timestamp: new Date().toISOString(),
      engine: 'MoFA Engine v0.1.0',
      telemetry: {
        session: {
          localRequests,
          cloudRequests,
          cloudCostUsd,
          warmupHits,
          localRatioPercent: localRatio,
        },
        allTime: {
          totalLocalRequests,
          totalCloudRequests,
          totalCloudCost,
          usdSavedByLocal,
        },
        hardware: {
          memUsedGb,
          memBudgetGb,
          memPercent,
        }
      }
    };

    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `mofa_telemetry_${Date.now()}.json`;
    a.click();
    URL.revokeObjectURL(url);
    setExported(true);
    setTimeout(() => setExported(false), 2000);
  };

  return (
    <div className="w-full space-y-5">
      {/* Top Session Bar & Export */}
      <div className="flex items-center justify-between text-[11px] font-mono text-text-dim px-1">
        <div className="flex items-center gap-3">
          <span>session active ({sessionDurationStr})</span>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => refresh()}
            className="px-2 py-1 rounded text-text-secondary hover:text-text-primary hover:bg-white/5 transition-colors cursor-pointer"
            title="Press 'R' to refresh"
          >
            Refresh (R)
          </button>
          <button
            onClick={handleExportTelemetry}
            className="flex items-center gap-1.5 px-2 py-1 rounded text-text-secondary hover:text-text-primary hover:bg-white/5 transition-colors cursor-pointer"
            title="Press 'E' to export"
          >
            <Download className="w-3.5 h-3.5" />
            <span>{exported ? 'Exported!' : 'Export JSON (E)'}</span>
          </button>
        </div>
      </div>

      {/* Sleek Live Telemetry Stream Ticker */}
      <TelemetryTicker />

      {/* Local vs Cloud Routing Ratio Bar */}
      <div className="bg-background-card border border-border-subtle rounded-[var(--radius-card)] p-3 shadow-md">
        <div className="flex items-center justify-between text-[11px] font-mono text-text-dim mb-2">
          <span>Routing Ratio (Session)</span>
          <span>Local: {localRatio}% / Cloud: {cloudRatio}%</span>
        </div>
        <div className="w-full bg-white/10 rounded-full h-1.5 flex overflow-hidden">
          <div 
            className="bg-accent-green h-full transition-all duration-500" 
            style={{ width: `${localRatio}%` }} 
            title="Local Hardware Traffic" 
          />
          <div 
            className="bg-orange-500 h-full transition-all duration-500" 
            style={{ width: `${cloudRatio}%` }} 
            title="Cloud Financial Traffic" 
          />
        </div>
      </div>

      {/* Side-by-Side Track Cards */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-5 items-stretch">
        <LatencyAvailabilityDashboard 
          localRequests={localRequests}
          memUsedGb={memUsedGb}
          memBudgetGb={memBudgetGb}
          memPercent={memPercent}
          totalLocalRequests={totalLocalRequests}
          warmupHits={warmupHits}
          historyData={localHistory.map(h => h.value)}
          lastUpdated={lastUpdated}
          isLoading={isLoading}
        />
        <CostTokenDashboard 
          cloudRequests={cloudRequests}
          cloudCostUsd={cloudCostUsd}
          totalCloudRequests={totalCloudRequests}
          totalCloudCost={totalCloudCost}
          quotaErrors={0}
          budgetCapUsd={1.00}
          historyData={costHistory.map(h => h.value)}
          lastUpdated={lastUpdated}
          isLoading={isLoading}
        />
      </div>

      {/* Real-time SVG Area Charts */}
      <SessionCharts 
        localHistory={velocityHistory && velocityHistory.length > 0 ? velocityHistory : localHistory} 
        costHistory={costHistory} 
        memHistory={memHistory} 
      />

      {/* Live Request Stream Activity Feed */}
      <ActivityFeed items={activityFeed} />

      {/* Dynamic Model Efficiency Matrix Table */}
      <ModelEfficiencyTable capabilities={capabilities} />
    </div>
  );
}
