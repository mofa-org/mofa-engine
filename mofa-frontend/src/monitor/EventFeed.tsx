import React, { useEffect, useState } from 'react';
import { engine } from '../engine';
import { EngineEvent } from '../engine/types';
import { Activity, CheckCircle2, Cpu, Loader, Box, RotateCw, Network, ShieldAlert } from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';

export function EventFeed() {
  const [events, setEvents] = useState<(EngineEvent & { _id: string })[]>([]);
  const [traceIdFilter, setTraceIdFilter] = useState('');

  useEffect(() => {
    let counter = 0;
    const handleEvent = (evt: EngineEvent) => {
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
            <EventCard key={evt._id} evt={evt} />
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

function EventCard({ evt }: { evt: EngineEvent }) {
  const { icon, color, title, desc } = formatEvent(evt);
  
  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: -10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2 }}
      className={`border border-border-subtle rounded-[var(--radius-small)] p-2.5 text-xs bg-background-card shadow-sm flex items-start gap-3 shrink-0 border-l-[3px] border-l-${color}`}
    >
      <div className={`mt-0.5 text-${color}`}>
        {icon}
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex justify-between items-start gap-2 mb-0.5">
          <span className={`font-medium text-${color} truncate`}>{title}</span>
          <span className="text-[10px] text-text-dim whitespace-nowrap shrink-0">
            {new Date(evt.timestamp).toLocaleTimeString([], { hour12: false, hour: '2-digit', minute: '2-digit', second:'2-digit' })}
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
