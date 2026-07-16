import React, { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { Card } from '../shared/Card';
import { Button } from '../shared/Button';
import { MetricsStrip } from './MetricsStrip';
import { Activity, LayoutDashboard, Cpu, Network, ExternalLink } from 'lucide-react';

const GRAFANA_URL = import.meta.env.VITE_GRAFANA_URL || 'http://localhost:3000';

type TabId = 'overview' | 'memory' | 'routing';

interface TabConfig {
  id: TabId;
  label: string;
  icon: React.ReactNode;
  dashboardPath: string;
  description: string;
}

const TABS: TabConfig[] = [
  {
    id: 'overview',
    label: 'Engine Overview',
    icon: <LayoutDashboard className="w-4 h-4" />,
    dashboardPath: '/d/engine-overview/mofa-engine-overview',
    description: 'Request rate, error rate, P95 latency, TTFT, and tokens/sec.'
  },
  {
    id: 'memory',
    label: 'Memory & Lifecycle',
    icon: <Cpu className="w-4 h-4" />,
    dashboardPath: '/d/engine-memory/mofa-memory-and-lifecycle',
    description: 'Memory vs budget, model loads/unloads, eviction rate, and cold-load heatmap.'
  },
  {
    id: 'routing',
    label: 'Preflight & Routing',
    icon: <Network className="w-4 h-4" />,
    dashboardPath: '/d/engine-routing/mofa-preflight-and-routing',
    description: 'Fallback routing, circuit breakers, and capability matching.'
  }
];

export function ObservabilityView() {
  const [activeTab, setActiveTab] = useState<TabId>('overview');
  const [grafanaAvailable, setGrafanaAvailable] = useState<boolean | null>(null);

  useEffect(() => {
    let mounted = true;
    
    // Lightweight check for Grafana availability using a known public static asset
    const checkGrafana = () => {
      const img = new Image();
      img.onload = () => {
        if (mounted) setGrafanaAvailable(true);
      };
      img.onerror = () => {
        if (mounted) setGrafanaAvailable(false);
      };
      // Prevent caching
      img.src = `${GRAFANA_URL}/public/img/grafana_icon.svg?t=${Date.now()}`;
    };

    checkGrafana();

    return () => { mounted = false; };
  }, []);

  const activeConfig = TABS.find(t => t.id === activeTab)!;

  return (
    <motion.div 
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -10 }}
      className="flex-1 w-full h-full overflow-y-auto bg-background-primary p-6"
    >
      <div className="max-w-[1200px] mx-auto flex flex-col min-h-full">
        {/* Header */}
        <div className="flex items-center justify-between mb-8">
          <div>
            <h1 className="text-[24px] font-semibold text-text-primary flex items-center gap-3">
              <Activity className="w-6 h-6 text-accent-blue" />
              Engine Observability
            </h1>
            <p className="text-[14px] text-text-dim mt-1 font-mono">
              Powered by Prometheus + Grafana
            </p>
          </div>
          {grafanaAvailable && (
            <Button 
              variant="secondary" 
              onClick={() => window.open(GRAFANA_URL, '_blank')}
              className="gap-2"
            >
              Open Grafana <ExternalLink className="w-4 h-4" />
            </Button>
          )}
        </div>

        <MetricsStrip />

        {/* Tab Navigation */}
        <div className="flex items-center gap-2 mb-6 border-b border-black/5 pb-px">
          {TABS.map(tab => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`flex items-center gap-2 px-4 py-2 text-[13px] font-medium transition-colors border-b-2 ${
                activeTab === tab.id 
                  ? 'border-accent-blue text-accent-blue' 
                  : 'border-transparent text-text-secondary hover:text-text-primary hover:border-black/10'
              }`}
            >
              {tab.icon}
              {tab.label}
            </button>
          ))}
        </div>

        <div className="flex-1 min-h-[800px]">
          {grafanaAvailable === null ? (
            <div className="w-full h-full flex items-center justify-center">
              <div className="text-text-dim animate-pulse text-sm">Checking observability stack...</div>
            </div>
          ) : grafanaAvailable ? (
            <Card className="w-full h-full overflow-hidden p-0 border-black/10">
              <div className="bg-background-secondary border-b border-black/5 px-4 py-3 flex items-center justify-between">
                <div className="text-[13px] font-medium text-text-primary">{activeConfig.label}</div>
                <div className="text-[11px] text-text-dim">{activeConfig.description}</div>
              </div>
              <div className="w-full h-[calc(100%-45px)] bg-black/5">
                <iframe
                  src={`${GRAFANA_URL}${activeConfig.dashboardPath}?kiosk&from=now-5m&to=now&refresh=5s`}
                  className="w-full h-full border-none"
                  title={activeConfig.label}
                />
              </div>
            </Card>
          ) : (
            <Card className="w-full h-full flex flex-col items-center justify-center text-center p-8 border-dashed border-black/10 bg-background-secondary/50">
              <div className="w-16 h-16 rounded-2xl bg-black/5 flex items-center justify-center mb-6">
                <Activity className="w-8 h-8 text-text-dim" />
              </div>
              <h3 className="text-lg font-medium text-text-primary mb-2">Grafana Not Reachable</h3>
              <p className="text-text-secondary text-sm max-w-md mb-6">
                Grafana dashboards appear here when the observability stack is running. 
                Currently, it is either not started or running on a different port.
              </p>
              <div className="p-4 bg-background-primary rounded-lg border border-black/5 text-left w-full max-w-md shadow-sm">
                <div className="text-[12px] font-medium text-text-secondary uppercase tracking-wider mb-2">Expected configuration</div>
                <div className="font-mono text-[11px] text-text-primary space-y-1">
                  <div><span className="text-text-dim">VITE_GRAFANA_URL=</span>{GRAFANA_URL}</div>
                  <div><span className="text-text-dim">Dashboard:</span> {activeConfig.dashboardPath}</div>
                </div>
              </div>
              <Button 
                variant="primary" 
                className="mt-8 gap-2"
                onClick={() => window.open(GRAFANA_URL, '_blank')}
              >
                Try Opening Grafana <ExternalLink className="w-4 h-4" />
              </Button>
            </Card>
          )}
        </div>
      </div>
    </motion.div>
  );
}
