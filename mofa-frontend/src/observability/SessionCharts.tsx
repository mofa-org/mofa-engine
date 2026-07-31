import React from 'react';
import { Card } from '../shared/Card';
import { AreaChart } from '../shared/AreaChart';

interface SessionChartsProps {
  localHistory: { timestamp: number; value: number }[];
  costHistory: { timestamp: number; value: number }[];
  memHistory: { timestamp: number; value: number }[];
}

export function SessionCharts({ localHistory, costHistory, memHistory }: SessionChartsProps) {
  return (
    <div className="grid grid-cols-1 md:grid-cols-3 gap-5">
      <Card className="p-5">
        <div className="flex items-center justify-between mb-3">
          <h4 className="text-[12px] font-semibold uppercase tracking-wider text-text-dim">
            Inference Velocity
          </h4>
          <span className="w-2 h-2 rounded-full bg-accent-green" />
        </div>
        <AreaChart 
          data={localHistory} 
          color="#22c55e" 
          gradientId="localReqsGrad" 
          unit=" reqs"
        />
      </Card>

      <Card className="p-5">
        <div className="flex items-center justify-between mb-3">
          <h4 className="text-[12px] font-semibold uppercase tracking-wider text-text-dim">
            Cloud Cost Curve
          </h4>
          <span className="w-2 h-2 rounded-full bg-orange-500" />
        </div>
        <AreaChart 
          data={costHistory} 
          color="#f97316" 
          gradientId="cloudCostGrad" 
          formatValue={(v) => `$${v.toFixed(5)}`}
        />
      </Card>

      <Card className="p-5">
        <div className="flex items-center justify-between mb-3">
          <h4 className="text-[12px] font-semibold uppercase tracking-wider text-text-dim">
            Memory Pressure
          </h4>
          <span className="w-2 h-2 rounded-full bg-accent-blue" />
        </div>
        <AreaChart 
          data={memHistory} 
          color="#3b82f6" 
          gradientId="memPressGrad" 
          unit="%"
        />
      </Card>
    </div>
  );
}
