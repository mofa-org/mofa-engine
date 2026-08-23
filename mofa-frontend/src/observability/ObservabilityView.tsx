import React, { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { Card } from '../shared/Card';
import { Button } from '../shared/Button';
import { MetricsStrip } from './MetricsStrip';
import { DualTrackView } from './DualTrackView';
import { useEngineMetrics } from './useEngineMetrics';
import { engine } from '../engine';
import { Activity, LayoutDashboard, Cpu, Network, ExternalLink, Layers, DollarSign, ShieldCheck } from 'lucide-react';
import { DataFlowAudit } from './DataFlowAudit';

const GRAFANA_URL = import.meta.env.VITE_GRAFANA_URL || 'http://localhost:3001';

type TabId = 'dual-track' | 'audit' | 'overview' | 'memory' | 'routing' | 'cost';

interface TabConfig {
  id: TabId;
  label: string;
  icon: React.ReactNode;
  dashboardPath: string;
  description: string;
}

const TABS: TabConfig[] = [
  {
    id: 'dual-track',
    label: 'Dual-Track Telemetry',
    icon: <Layers className="w-4 h-4" />,
    dashboardPath: '',
    description: 'Real-time side-by-side comparison of local hardware footprint vs cloud financial cost.'
  },
  {
    id: 'audit',
    label: 'Data Flow Audit',
    icon: <ShieldCheck className="w-4 h-4" />,
    dashboardPath: '',
    description: 'Privacy compliance audit — verify sensitive data never hits cloud endpoints.'
  },
  {
    id: 'overview',
    label: 'Engine Overview',
    icon: <LayoutDashboard className="w-4 h-4" />,
    dashboardPath: '/d/engine-overview',
    description: 'Request rate, error rate, P95 latency, TTFT, and tokens/sec.'
  },
  {
    id: 'memory',
    label: 'Memory & Lifecycle',
    icon: <Cpu className="w-4 h-4" />,
    dashboardPath: '/d/engine-memory',
    description: 'Memory vs budget, model loads/unloads, eviction rate, and cold-load heatmap.'
  },
  {
    id: 'routing',
    label: 'Preflight & Routing',
    icon: <Network className="w-4 h-4" />,
    dashboardPath: '/d/engine-routing',
    description: 'Fallback routing, circuit breakers, and capability matching.'
  },
  {
    id: 'cost',
    label: 'Financial Cost & Billing',
    icon: <DollarSign className="w-4 h-4" />,
    dashboardPath: '/d/engine-cost',
    description: 'Real-time USD spend accumulation, free-tier savings rate %, token velocity, and billing rate per minute.'
  }
];

export function ObservabilityView() {
  const metrics = useEngineMetrics();
  const [activeTab, setActiveTab] = useState<TabId>('dual-track');
  const [grafanaAvailable, setGrafanaAvailable] = useState<boolean | null>(null);
  const [engineMeta, setEngineMeta] = useState<{ version: string; uptime: number; providers: number } | null>(null);

  useEffect(() => {
    let mounted = true;
    
    const checkGrafana = () => {
      const img = new Image();
      img.onload = () => {
        if (mounted) setGrafanaAvailable(true);
      };
      img.onerror = () => {
        if (mounted) setGrafanaAvailable(false);
      };
      img.src = `${GRAFANA_URL}/public/img/grafana_icon.svg?t=${Date.now()}`;
    };

    const fetchEngineMeta = () => {
      Promise.all([engine.getHealth(), engine.getStatus()]).then(([healthRes, statusRes]) => {
        if (mounted && healthRes.success) {
          setEngineMeta({
            version: healthRes.data.version || '0.1.0',
            uptime: healthRes.data.uptime_secs || 0,
            providers: statusRes.success ? statusRes.data.providers : 3
          });
        }
      });
    };

    checkGrafana();
    fetchEngineMeta();

    return () => { mounted = false; };
  }, []);

  // Power user Keyboard Shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (['input', 'textarea'].includes((e.target as HTMLElement)?.tagName?.toLowerCase())) return;
      if (e.key === '1') setActiveTab('dual-track');
      else if (e.key === '2') setActiveTab('overview');
      else if (e.key === '3') setActiveTab('memory');
      else if (e.key === '4') setActiveTab('routing');
      else if (e.key === '5') setActiveTab('cost');
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  const activeConfig = TABS.find(t => t.id === activeTab)!;

  const uptimeMins = engineMeta ? Math.floor(engineMeta.uptime / 60) : 0;

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
              MoFA Engine v{engineMeta?.version || '0.1.0'} · Uptime {uptimeMins}m · {engineMeta?.providers || 3} active providers
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

        <MetricsStrip
          status={metrics.status}
          totalRequestCount={metrics.totalRequestCount}
          p50={metrics.p50}
          p95={metrics.p95}
          p99={metrics.p99}
          usdSavedByLocal={metrics.usdSavedByLocal}
        />

        {/* Tab Navigation */}
        <div className="flex items-center gap-2 mb-6 border-b border-border-subtle pb-px flex-wrap">
          {TABS.map(tab => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`flex items-center gap-2 px-4 py-2 text-[13px] font-medium transition-colors border-b-2 ${
                activeTab === tab.id 
                  ? 'border-accent-blue text-accent-blue' 
                  : 'border-transparent text-text-secondary hover:text-text-primary hover:border-border-strong'
              }`}
            >
              {tab.icon}
              {tab.label}
            </button>
          ))}
        </div>

        <div className="flex-1 min-h-[600px]">
          {activeTab === 'dual-track' ? (
            <DualTrackView />
          ) : activeTab === 'audit' ? (
            <DataFlowAudit />
          ) : grafanaAvailable === null ? (
            <div className="w-full h-full flex items-center justify-center">
              <div className="text-text-dim animate-pulse text-sm">Checking observability stack...</div>
            </div>
          ) : grafanaAvailable ? (
            <Card className="w-full h-full overflow-hidden p-0 border-border-strong">
              <div className="bg-background-secondary border-b border-border-subtle px-4 py-3 flex items-center justify-between">
                <div className="text-[13px] font-medium text-text-primary">{activeConfig.label}</div>
                <div className="text-[11px] text-text-dim">{activeConfig.description}</div>
              </div>
              <div className="w-full h-[calc(100%-45px)] min-h-[600px] bg-background-hover">
                <iframe
                  src={`${GRAFANA_URL}${activeConfig.dashboardPath}?kiosk&from=now-5m&to=now&refresh=5s`}
                  className="w-full h-full border-none min-h-[600px]"
                  title={activeConfig.label}
                />
              </div>
            </Card>
          ) : (
            <Card className="w-full h-full flex flex-col items-center justify-center text-center p-8 border-dashed border-border-strong bg-background-secondary/50">
              <div className="w-16 h-16 rounded-2xl bg-background-hover flex items-center justify-center mb-6">
                <Activity className="w-8 h-8 text-text-dim" />
              </div>
              <h3 className="text-lg font-medium text-text-primary mb-2">Grafana Not Reachable</h3>
              <p className="text-text-secondary text-sm max-w-md mb-6">
                Grafana dashboards appear here when the observability stack is running. 
                Currently, it is either not started or running on a different port.
              </p>
              <div className="p-4 bg-background-card rounded-lg border border-border-subtle text-left w-full max-w-md shadow-sm">
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
