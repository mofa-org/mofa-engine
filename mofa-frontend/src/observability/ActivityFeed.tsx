import React from 'react';
import { Card } from '../shared/Card';
import { TelemetryLogItem } from './useEngineMetrics';
import { motion, AnimatePresence } from 'framer-motion';

interface ActivityFeedProps {
  items: TelemetryLogItem[];
}

export function ActivityFeed({ items }: ActivityFeedProps) {
  return (
    <Card className="p-6">
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-2">
          <span className="w-2 h-2 rounded-full bg-accent-green animate-pulse" />
          <h4 className="text-[12px] font-semibold uppercase tracking-wider text-text-dim">
            Live Request Stream
          </h4>
        </div>
        <span className="text-[11px] font-mono text-text-dim">
          {items.length} events buffered
        </span>
      </div>

      <div className="max-h-[220px] overflow-y-auto pr-1 space-y-2">
        <AnimatePresence initial={false}>
          {items.length === 0 ? (
            <div className="text-[12px] font-mono text-text-dim text-center py-8 border border-dashed border-border-subtle rounded-md">
              no inference activity yet — submit a prompt to populate stream
            </div>
          ) : (
            items.map((item) => (
              <motion.div
                key={item.id}
                initial={{ opacity: 0, height: 0, y: -10 }}
                animate={{ opacity: 1, height: 'auto', y: 0 }}
                exit={{ opacity: 0, height: 0 }}
                transition={{ duration: 0.2 }}
                className="flex items-center justify-between text-[12px] p-2.5 rounded bg-background-secondary border border-border-subtle hover:border-border-strong transition-colors font-mono"
              >
                <div className="flex items-center gap-3">
                  <span className="text-text-dim text-[11px]">
                    {new Date(item.timestamp).toLocaleTimeString()}
                  </span>
                  <span className="font-sans font-medium text-text-primary">
                    {item.model.split('/').pop()}
                  </span>
                  <span className={`text-[10px] px-1.5 py-0.5 rounded border font-semibold ${
                    item.locality === 'local'
                      ? 'bg-accent-green/10 border-accent-green/20 text-accent-green'
                      : 'bg-orange-500/10 border-orange-500/20 text-orange-400'
                  }`}>
                    {item.locality}
                  </span>
                  {item.isFallback && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-accent-red/10 border border-accent-red/20 text-accent-red">
                      fallback
                    </span>
                  )}
                </div>

                <div className="flex items-center gap-4 text-text-secondary">
                  <span>{item.tokensUsed} tokens</span>
                  <span className="font-semibold text-text-primary">
                    {item.durationMs > 0 ? `${(item.durationMs / 1000).toFixed(2)}s` : '<10ms'}
                  </span>
                  <span className="text-text-dim min-w-[50px] text-right">
                    {item.costUsd > 0 ? `$${item.costUsd.toFixed(5)}` : 'free'}
                  </span>
                </div>
              </motion.div>
            ))
          )}
        </AnimatePresence>
      </div>
    </Card>
  );
}
