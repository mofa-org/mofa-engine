import React, { useEffect, useState } from 'react';
import { engine } from '../engine';
import { EngineEvent } from '../engine/types';
import { Activity, CheckCircle2, Cpu, Loader, Box, RotateCw, Network, ShieldAlert } from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';

export function EventFeed() {
  const [events, setEvents] = useState<(EngineEvent & { _id: string })[]>([]);
  const [traceIdFilter, setTraceIdFilter] = useState('');
  const [now, setNow] = useState(Date.now());

  // Real-time second ticker for live relative time
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    let counter = 0;
    let lastMemBytes: number | null = null;

    const handleEvent = (evt: EngineEvent) => {
      // Deduplicate identical consecutive MemoryChanged events
      if (evt.type === 'MemoryChanged') {
        const bytes = (evt.data as any)?.used_bytes;
        if (bytes !== undefined && bytes === lastMemBytes) {
          return;
        }
        lastMemBytes = bytes;
      }

      setEvents(prev => {
        const newEvt = { ...evt, _id: `${evt.timestamp}-${counter++}` };
        const next = [newEvt, ...prev]; // Prepend for reverse chronological
        if (next.length > 15) return next.slice(0, 15);
        return next;
      });
    };

    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => unsubscribe();
  }, []);

  const filteredEvents = traceIdFilter 
    ? events.filter(e => e.data.trace_id?.includes(traceIdFilter) || e.data.request_id?.includes(traceIdFilter))
    : events;

  return (
    <div className="flex-1 overflow-hidden flex flex-col relative min-h-0">
      <div className="p-2 shrink-0 border-b border-border-subtle">
        <input 
          type="text" 
          placeholder="Filter by trace ID..." 
          className="w-full text-xs px-2 py-1 rounded bg-background-hover border-none focus:ring-1 focus:ring-accent-blue outline-none text-text-primary"
          value={traceIdFilter}
          onChange={(e) => setTraceIdFilter(e.target.value)}
        />
      </div>
      <div className="flex-1 overflow-y-auto overflow-x-hidden flex flex-col gap-2 p-2">
        <AnimatePresence initial={false}>
          {filteredEvents.map((evt) => (
            <EventCard key={evt._id} evt={evt} now={now} />
          ))}
        </AnimatePresence>
        {events.length === 0 && (
          <div className="text-text-dim italic text-[11px] w-full text-center mt-4">Waiting for engine events...</div>
        )}
      </div>
      <div className="absolute top-[45px] inset-x-0 h-4 bg-gradient-to-b from-background-secondary to-transparent pointer-events-none" />
      <div className="absolute bottom-0 inset-x-0 h-4 bg-gradient-to-t from-background-secondary to-transparent pointer-events-none" />
    </div>
  );
}

function formatRelativeTime(ts: number, now: number): string {
  const diffSec = Math.max(0, Math.floor((now - ts) / 1000));
  if (diffSec < 3) return 'just now';
  if (diffSec < 60) return `${diffSec}s ago`;
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  return new Date(ts).toLocaleTimeString([], { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

const COLOR_STYLES: Record<string, { border: string; text: string }> = {
  'accent-yellow': { border: 'border-l-accent-yellow', text: 'text-accent-yellow' },
  'accent-purple': { border: 'border-l-accent-purple', text: 'text-accent-purple' },
  'accent-red': { border: 'border-l-accent-red', text: 'text-accent-red' },
  'accent-blue': { border: 'border-l-accent-blue', text: 'text-accent-blue' },
  'accent-green': { border: 'border-l-accent-green', text: 'text-accent-green' },
  'accent-cyan': { border: 'border-l-accent-cyan', text: 'text-accent-cyan' },
  'text-dim': { border: 'border-l-text-dim', text: 'text-text-dim' },
};

function EventCard({ evt, now }: { evt: EngineEvent; now: number }) {
  const { icon, color, title, desc } = formatEvent(evt);
  const timeStr = new Date(evt.timestamp).toLocaleTimeString([], { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
  const styles = COLOR_STYLES[color] || { border: 'border-l-text-dim', text: 'text-text-dim' };
  
  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: -10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2 }}
      className={`border border-border-subtle rounded-[var(--radius-small)] p-2.5 text-xs bg-background-card shadow-sm flex items-start gap-3 shrink-0 border-l-[3px] ${styles.border}`}
    >
      <div className={`mt-0.5 ${styles.text}`}>
        {icon}
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex justify-between items-start gap-2 mb-0.5">
          <span className={`font-medium truncate ${styles.text}`}>{title}</span>
          <span 
            className="text-[10px] font-mono text-text-dim whitespace-nowrap shrink-0 hover:text-text-secondary cursor-default"
            title={`Event recorded at ${timeStr}`}
          >
            {formatRelativeTime(evt.timestamp, now)}
          </span>
        </div>
        <div className="text-text-secondary leading-snug">
          {desc}
        </div>
      </div>
    </motion.div>
  );
}

function formatEvent(evt: EngineEvent) {
  switch (evt.type) {
    case 'RoutingDecision':
      return {
        icon: <Network className="w-3.5 h-3.5" />,
        color: evt.data.is_fallback ? 'accent-yellow' : 'accent-purple',
        title: evt.data.is_fallback ? 'Routing (Fallback)' : 'Routing Decision',
        desc: `${evt.data.selected_backend}/${evt.data.selected_model} (${evt.data.reason || 'capability_match'})`
      };
    case 'FailoverTriggered':
      return {
        icon: <ShieldAlert className="w-3.5 h-3.5" />,
        color: 'accent-red',
        title: 'Failover Triggered',
        desc: `Failed ${evt.data.failed_backend} → ${evt.data.fallback_backend}`
      };
    case 'RequestStarted':
      return {
        icon: <Activity className="w-3.5 h-3.5" />,
        color: 'accent-blue',
        title: 'Request Started',
        desc: `${evt.data.capability || 'inference'} → ${evt.data.model_id}`
      };
    case 'RequestCompleted':
      return {
        icon: <CheckCircle2 className="w-3.5 h-3.5" />,
        color: evt.data.success ? 'accent-green' : 'accent-red',
        title: evt.data.success ? 'Request Completed' : 'Request Failed',
        desc: `${evt.data.trace_id?.slice(0, 8) || evt.data.request_id?.slice(0, 8)}… ${evt.data.duration_ms}ms`
      };
    case 'ModelStatusChanged':
      return {
        icon: <Loader className="w-3.5 h-3.5" />,
        color: 'accent-yellow',
        title: 'Model Status',
        desc: `${evt.data.model_id}: ${evt.data.old} → ${evt.data.new}`
      };
    case 'ModelResidencyChanged':
      return {
        icon: <Box className="w-3.5 h-3.5" />,
        color: 'accent-cyan',
        title: 'Model Residency',
        desc: `${evt.data.model_id}: ${evt.data.old} → ${evt.data.new}`
      };
    case 'MemoryChanged':
      return {
        icon: <Cpu className="w-3.5 h-3.5" />,
        color: 'accent-purple',
        title: 'Memory Changed',
        desc: `${(evt.data.used_bytes / 1024 / 1024 / 1024).toFixed(2)} GB / ${(evt.data.total_bytes / 1024 / 1024 / 1024).toFixed(1)} GB`
      };
    case 'ProviderHealthChanged':
      return {
        icon: <RotateCw className="w-3.5 h-3.5" />,
        color: 'accent-yellow',
        title: 'Provider Health',
        desc: `${evt.data.provider}: ${evt.data.health}`
      };
    case 'DiscoveryCompleted':
      return {
        icon: <Activity className="w-3.5 h-3.5" />,
        color: evt.data.success ? 'accent-green' : 'accent-red',
        title: 'Discovery',
        desc: `${evt.data.provider}: ${evt.data.models} model(s)`
      };
    default:
      return {
        icon: <Activity className="w-3.5 h-3.5" />,
        color: 'text-dim',
        title: evt.type || 'Event',
        desc: JSON.stringify(evt.data)
      };
  }
}
