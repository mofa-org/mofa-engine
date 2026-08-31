import React, { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Terminal, Filter, ArrowRightLeft, ShieldAlert } from 'lucide-react';
import { engine } from '../engine';
import { Badge } from '../shared/Badge';

interface RoutingEvent {
  id: string;
  timestamp: Date;
  capability: string;
  selected_model: string;
  selected_backend: string;
  is_fallback: boolean;
  reason: string;
}

export function RoutingDecisionLog() {
  const [events, setEvents] = useState<RoutingEvent[]>([]);
  const [filterFallback, setFilterFallback] = useState(false);

  useEffect(() => {
    const handleEvent = (evt: any) => {
      try {
        if (evt?.type === 'RoutingDecision') {
          const d = evt?.data || {};
          const newEvent: RoutingEvent = {
            id: Math.random().toString(36).substring(7),
            timestamp: new Date(),
            capability: d.capability || 'chat',
            selected_model: d.selected_model || 'unknown',
            selected_backend: d.selected_backend || 'local',
            is_fallback: !!d.is_fallback,
            reason: d.reason || 'capability_match',
          };
          
          setEvents(prev => {
            const updated = [newEvent, ...prev];
            // Keep only the last 100 events to prevent memory bloat
            return updated.slice(0, 100);
          });
        }
      } catch (err) {
        console.warn('Failed to parse RoutingDecision event', err);
      }
    };

    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => unsubscribe();
  }, []);

  const displayedEvents = filterFallback ? events.filter(e => e.is_fallback) : events;

  return (
    <div className="bg-background-card border border-border-subtle rounded-xl overflow-hidden flex flex-col h-[400px]">
      <div className="flex items-center justify-between p-3 border-b border-border-subtle bg-white/10">
        <div className="flex items-center gap-2 text-text-primary text-[13px] font-semibold">
          <Terminal className="w-4 h-4 text-accent-blue" />
          Live Routing Decision Log
        </div>
        <button
          onClick={() => setFilterFallback(!filterFallback)}
          className={`flex items-center gap-1 text-[11px] px-2 py-1 rounded transition-colors ${
            filterFallback 
              ? 'bg-accent-yellow/20 text-accent-yellow' 
              : 'bg-white/10 text-text-secondary hover:text-text-primary'
          }`}
        >
          <Filter className="w-3 h-3" />
          {filterFallback ? 'Fallback Only' : 'All Requests'}
        </button>
      </div>
      
      <div className="flex-1 overflow-y-auto p-3 space-y-2 font-mono text-[11px]">
        <AnimatePresence initial={false}>
          {displayedEvents.map(evt => (
            <motion.div
              key={evt.id}
              initial={{ opacity: 0, y: -10 }}
              animate={{ opacity: 1, y: 0 }}
              className={`p-2 rounded border ${
                evt.is_fallback 
                  ? 'border-accent-yellow/30 bg-accent-yellow/5 text-accent-yellow' 
                  : 'border-border-subtle bg-background-secondary text-text-secondary'
              }`}
            >
              <div className="flex items-center gap-2 mb-1">
                <span className="text-text-dim">{evt.timestamp.toLocaleTimeString()}</span>
                <Badge variant={evt.is_fallback ? 'yellow' : 'blue'}>{evt.capability}</Badge>
                {evt.is_fallback && <ShieldAlert className="w-3 h-3 ml-auto" />}
              </div>
              <div className="flex items-center gap-2 text-[12px] text-text-primary mt-1">
                <span className="text-accent-blue">{evt.selected_backend}</span>
                <span className="text-text-dim">/</span>
                <span>{evt.selected_model}</span>
                <ArrowRightLeft className="w-3 h-3 mx-1 text-text-dim" />
                <span className="truncate text-text-dim" title={evt.reason}>
                  {evt.reason}
                </span>
              </div>
            </motion.div>
          ))}
          {displayedEvents.length === 0 && (
            <div className="text-center text-text-dim py-8">
              No routing events yet. Waiting for requests...
            </div>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
}
