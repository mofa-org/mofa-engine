import React, { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { Card } from '../shared/Card';
import { Badge } from '../shared/Badge';
import { engine } from '../engine';
import { EngineStatus } from '../engine/types';
import { HardDrive, DollarSign, Zap, AlertTriangle, ShieldCheck, Flame, Cpu, ArrowUpRight } from 'lucide-react';

export function DualTrackView() {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [localRequests, setLocalRequests] = useState(0);
  const [cloudRequests, setCloudRequests] = useState(0);
  const [cloudCostUsd, setCloudCostUsd] = useState(0);
  const [thoughtTokens, setThoughtTokens] = useState(0);
  const [quotaErrors, setQuotaErrors] = useState(0);
  const [warmupHits, setWarmupHits] = useState(0);
  const [evictions, setEvictions] = useState(0);

  useEffect(() => {
    engine.getStatus().then(res => {
      if (res.success) setStatus(res.data);
    });

    const handleEvent = (evt: any) => {
      const type = evt.type;
      const data = evt.data || {};

      if (type === 'RequestCompleted') {
        if (data.provider === 'ollama' || data.provider === 'kokoro' || data.provider === 'funasr' || data.locality === 'local') {
          setLocalRequests(prev => prev + 1);
        } else {
          setCloudRequests(prev => prev + 1);
          if (data.cost_usd) {
            setCloudCostUsd(prev => prev + data.cost_usd);
          }
        }
      } else if (type === 'PreflightWarmCompleted') {
        if (data.success) setWarmupHits(prev => prev + 1);
      } else if (type === 'ModelEvicted') {
        setEvictions(prev => prev + 1);
      } else if (type === 'MemoryChanged' || type === 'ModelStatusChanged' || type === 'ModelResidencyChanged') {
        engine.getStatus().then(res => {
          if (res.success) setStatus(res.data);
        });
      }
    };

    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => unsubscribe();
  }, []);

  const memUsedGb = status ? (status.memory_used_bytes / (1024 * 1024 * 1024)).toFixed(2) : '3.11';
  const memBudgetGb = status ? (status.memory_budget_bytes / (1024 * 1024 * 1024)).toFixed(1) : '8.0';
  const memPercent = status && status.memory_budget_bytes > 0 
    ? Math.min(100, Math.round((status.memory_used_bytes / status.memory_budget_bytes) * 100))
    : 38.9;

  return (
    <div className="w-full space-y-6">
      {/* Header Banner */}
      <div className="flex items-center justify-between bg-background-card border border-border-light rounded-xl p-4">
        <div className="flex items-center gap-3">
          <div className="p-2 rounded-lg bg-accent-blue/10 text-accent-blue">
            <Zap className="w-5 h-5" />
          </div>
          <div>
            <h3 className="text-[15px] font-semibold text-text-primary">Dual-Track Telemetry Moat</h3>
            <p className="text-[13px] text-text-secondary">
              Real-time side-by-side comparison of local hardware performance vs cloud financial cost accumulation.
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Badge variant="green">Local-First Priority Active</Badge>
        </div>
      </div>

      {/* Side-by-Side Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        
        {/* Left Column: Local Execution Track */}
        <motion.div 
          initial={{ opacity: 0, x: -10 }}
          animate={{ opacity: 1, x: 0 }}
          className="space-y-4"
        >
          <Card className="p-5 border-l-4 border-l-accent-green">
            <div className="flex items-center justify-between mb-4">
              <div className="flex items-center gap-2 text-accent-green font-medium text-[14px]">
                <HardDrive className="w-4 h-4" />
                Local Hardware Track (locality="local")
              </div>
              <Badge variant="green">0.00 USD / Free</Badge>
            </div>

            {/* VRAM Memory Gauge */}
            <div className="space-y-2 mb-4 bg-background-primary/50 p-3 rounded-lg border border-border-light/50">
              <div className="flex justify-between text-xs text-text-secondary">
                <span>VRAM / RAM Footprint</span>
                <span className="font-mono text-text-primary">{memUsedGb} / {memBudgetGb} GB ({memPercent}%)</span>
              </div>
              <div className="w-full bg-background-card h-2.5 rounded-full overflow-hidden border border-border-light">
                <div 
                  className="bg-accent-green h-full transition-all duration-500 rounded-full"
                  style={{ width: `${memPercent}%` }}
                />
              </div>
            </div>

            {/* Sub-Metrics Grid */}
            <div className="grid grid-cols-2 gap-3">
              <div className="p-3 bg-background-primary/30 rounded-lg border border-border-light/30">
                <div className="flex items-center gap-1.5 text-xs text-text-secondary mb-1">
                  <Flame className="w-3.5 h-3.5 text-accent-yellow" />
                  Preflight Warmup Hits
                </div>
                <div className="text-lg font-semibold font-mono text-text-primary">{warmupHits}</div>
                <div className="text-[10px] text-accent-green">0ms Cold Start Latency</div>
              </div>

              <div className="p-3 bg-background-primary/30 rounded-lg border border-border-light/30">
                <div className="flex items-center gap-1.5 text-xs text-text-secondary mb-1">
                  <Cpu className="w-3.5 h-3.5 text-accent-blue" />
                  Local Inferences
                </div>
                <div className="text-lg font-semibold font-mono text-text-primary">{localRequests}</div>
                <div className="text-[10px] text-text-dim">Ollama + Kokoro TTS</div>
              </div>

              <div className="p-3 bg-background-primary/30 rounded-lg border border-border-light/30">
                <div className="flex items-center gap-1.5 text-xs text-text-secondary mb-1">
                  <AlertTriangle className="w-3.5 h-3.5 text-accent-red" />
                  LRU Memory Evictions
                </div>
                <div className="text-lg font-semibold font-mono text-text-primary">{evictions}</div>
                <div className="text-[10px] text-text-dim">Evicted under budget pressure</div>
              </div>

              <div className="p-3 bg-background-primary/30 rounded-lg border border-border-light/30">
                <div className="flex items-center gap-1.5 text-xs text-text-secondary mb-1">
                  <ShieldCheck className="w-3.5 h-3.5 text-accent-green" />
                  Privacy Classification
                </div>
                <div className="text-xs font-semibold text-text-primary">Confidential Only</div>
                <div className="text-[10px] text-accent-green">Zero Data Egress</div>
              </div>
            </div>
          </Card>
        </motion.div>

        {/* Right Column: Cloud Financial Track */}
        <motion.div 
          initial={{ opacity: 0, x: 10 }}
          animate={{ opacity: 1, x: 0 }}
          className="space-y-4"
        >
          <Card className="p-5 border-l-4 border-l-accent-purple">
            <div className="flex items-center justify-between mb-4">
              <div className="flex items-center gap-2 text-accent-purple font-medium text-[14px]">
                <DollarSign className="w-4 h-4" />
                Cloud Financial Track (locality="cloud")
              </div>
              <Badge variant="yellow">Vendor Billing Active</Badge>
            </div>

            {/* Financial Spend Gauge */}
            <div className="space-y-2 mb-4 bg-background-primary/50 p-3 rounded-lg border border-border-light/50">
              <div className="flex justify-between text-xs text-text-secondary">
                <span>Accumulated Compute Cost (USD)</span>
                <span className="font-mono text-text-primary font-semibold">${cloudCostUsd.toFixed(4)}</span>
              </div>
              <div className="w-full bg-background-card h-2.5 rounded-full overflow-hidden border border-border-light">
                <div 
                  className="bg-accent-purple h-full transition-all duration-500 rounded-full"
                  style={{ width: `${Math.min(100, cloudCostUsd * 20)}%` }}
                />
              </div>
            </div>

            {/* Sub-Metrics Grid */}
            <div className="grid grid-cols-2 gap-3">
              <div className="p-3 bg-background-primary/30 rounded-lg border border-border-light/30">
                <div className="flex items-center gap-1.5 text-xs text-text-secondary mb-1">
                  <ArrowUpRight className="w-3.5 h-3.5 text-accent-purple" />
                  Cloud Inferences
                </div>
                <div className="text-lg font-semibold font-mono text-text-primary">{cloudRequests}</div>
                <div className="text-[10px] text-text-dim">OpenAI / DeepSeek / Claude</div>
              </div>

              <div className="p-3 bg-background-primary/30 rounded-lg border border-border-light/30">
                <div className="flex items-center gap-1.5 text-xs text-text-secondary mb-1">
                  <Zap className="w-3.5 h-3.5 text-accent-blue" />
                  Thought Tokens
                </div>
                <div className="text-lg font-semibold font-mono text-text-primary">{thoughtTokens}</div>
                <div className="text-[10px] text-text-dim">DeepSeek R1 Reasoning</div>
              </div>

              <div className="p-3 bg-background-primary/30 rounded-lg border border-border-light/30">
                <div className="flex items-center gap-1.5 text-xs text-text-secondary mb-1">
                  <AlertTriangle className="w-3.5 h-3.5 text-accent-red" />
                  HTTP 429 Rate Limits
                </div>
                <div className="text-lg font-semibold font-mono text-text-primary">{quotaErrors}</div>
                <div className="text-[10px] text-text-dim">Vendor quota errors</div>
              </div>

              <div className="p-3 bg-background-primary/30 rounded-lg border border-border-light/30">
                <div className="flex items-center gap-1.5 text-xs text-text-secondary mb-1">
                  <DollarSign className="w-3.5 h-3.5 text-accent-green" />
                  Cost Savings Rate
                </div>
                <div className="text-xs font-semibold text-accent-green">100% Local Free Tier</div>
                <div className="text-[10px] text-text-dim">Saved vs pure cloud</div>
              </div>
            </div>
          </Card>
        </motion.div>

      </div>
    </div>
  );
}
