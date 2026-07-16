import React from 'react';
import { Activity, Shield, Box, Zap } from 'lucide-react';
import { MemoryGauge } from './MemoryGauge';
import { ModelGrid } from './ModelGrid';
import { ProviderHealth } from './ProviderHealth';
import { PreflightIndicator } from './PreflightIndicator';

export function MonitorSidebar() {
  return (
    <aside className="w-[280px] lg:w-[300px] bg-background-primary border-l border-black/5 flex flex-col shrink-0 hidden md:flex h-[calc(100vh-64px)] sticky top-16 overflow-y-auto">
      <div className="flex-1 p-5 flex flex-col gap-6">
        
        {/* Memory Constraints */}
        <section>
          <div className="flex items-center gap-2 mb-3">
            <Activity className="w-3.5 h-3.5 text-accent-purple" />
            <h3 className="text-[10px] font-semibold uppercase tracking-widest text-text-dim">Engine Memory</h3>
          </div>
          <div className="p-4 bg-background-secondary border border-black/5 rounded-md shadow-sm">
            <MemoryGauge />
          </div>
        </section>

        {/* Preflight & Activity */}
        <section>
          <div className="flex items-center gap-2 mb-3">
            <Zap className="w-3.5 h-3.5 text-accent-cyan" />
            <h3 className="text-[10px] font-semibold uppercase tracking-widest text-text-dim">Engine Activity</h3>
          </div>
          <PreflightIndicator />
        </section>

        {/* Provider Health */}
        <section>
          <div className="flex items-center gap-2 mb-2">
            <Shield className="w-3.5 h-3.5 text-accent-blue" />
            <h3 className="text-[10px] font-semibold uppercase tracking-widest text-text-dim">Provider Health</h3>
          </div>
          <div className="bg-background-secondary border border-black/5 rounded-md shadow-sm p-1">
            <ProviderHealth />
          </div>
        </section>

        {/* Active Models */}
        <section>
          <div className="flex items-center gap-2 mb-2">
            <Box className="w-3.5 h-3.5 text-accent-yellow" />
            <h3 className="text-[10px] font-semibold uppercase tracking-widest text-text-dim">Active Models</h3>
          </div>
          <div className="bg-background-secondary border border-black/5 rounded-md shadow-sm p-1">
            <ModelGrid />
          </div>
        </section>

      </div>
    </aside>
  );
}
