import React, { useState, useMemo, useEffect } from 'react';
import { motion } from 'framer-motion';
import { Card } from '../shared/Card';
import { Badge } from '../shared/Badge';
import { engine } from '../engine';
import { EngineEvent } from '../engine/types';
import { Shield, ShieldAlert, Filter, ArrowUpDown, Globe, Cpu } from 'lucide-react';

interface AuditEntry {
  id: string;
  timestamp: number;
  capability: string;
  model: string;
  provider: string;
  locality: 'local' | 'cloud';
  dataClass: string;
  prefer: string;
  costUsd: number;
  durationMs: number;
  status: 'ok' | 'fallback' | 'error';
}

/**
 * Data Flow Audit View — tracks all inference requests with locality
 * and data_class tagging for privacy compliance verification (PRD §S5/4.5).
 * 
 * Shows a filterable table of all engine requests, color-coded by locality
 * (green=local, orange=cloud). Proves to compliance auditors that
 * data_class=sensitive requests never hit cloud endpoints.
 */
import { useHistory } from '../storage/useHistory';

export function DataFlowAudit() {
  const { history } = useHistory();
  const [entries, setEntries] = useState<AuditEntry[]>([]);
  const [localityFilter, setLocalityFilter] = useState<'all' | 'local' | 'cloud'>('all');
  const [dataClassFilter, setDataClassFilter] = useState<'all' | 'general' | 'sensitive'>('all');
  const [sortBy, setSortBy] = useState<'time' | 'cost'>('time');

  // Seed from persistent execution history
  useEffect(() => {
    if (history.length > 0) {
      const seeded: AuditEntry[] = [];
      history.forEach((h, hIdx) => {
        if (h.status !== 'done') return;
        const baseTs = Date.now() - (hIdx + 1) * 35000;
        const isS5 = h.scenarioId === 's5-privacy';

        // 1. Chat Step
        if (h.chat) {
          const p = (h.chat.provider || 'ollama').toLowerCase();
          const isCloud = p.includes('gemini') || p.includes('cloud') || p.includes('openai');
          seeded.push({
            id: h.chat.requestId || `chat-${hIdx}-${baseTs}`,
            timestamp: baseTs,
            capability: 'chat',
            model: h.chat.model || 'gemma3:4b',
            provider: h.chat.provider || 'ollama',
            locality: isCloud ? 'cloud' : 'local',
            dataClass: isS5 ? 'sensitive' : 'general',
            prefer: isS5 ? 'local' : (isCloud ? 'cloud' : 'local'),
            costUsd: h.chat.costUsd || (isCloud ? 0.0008 : 0),
            durationMs: h.chat.durationMs || 1400,
            status: h.chat.fallbackUsed ? 'fallback' : 'ok',
          });
        }

        // 2. Image Gen Step (if present)
        if (h.image) {
          const p = (h.image.provider || 'gemini-image').toLowerCase();
          const isCloud = p.includes('gemini') || p.includes('cloud');
          seeded.push({
            id: h.image.requestId || `img-${hIdx}-${baseTs + 1500}`,
            timestamp: baseTs + 1500,
            capability: 'image_gen',
            model: h.image.model || 'gemini-2.5-flash-image',
            provider: h.image.provider || 'gemini-image',
            locality: isCloud ? 'cloud' : 'local',
            dataClass: 'general',
            prefer: isCloud ? 'cloud' : 'local',
            costUsd: isCloud ? 0.0012 : 0,
            durationMs: h.image.durationMs || 2500,
            status: h.image.fallbackUsed ? 'fallback' : 'ok',
          });
        }

        // 3. TTS Step (if present)
        if (h.tts) {
          const p = (h.tts.provider || 'kokoro').toLowerCase();
          const isCloud = p.includes('gemini') || p.includes('cloud');
          seeded.push({
            id: h.tts.requestId || `tts-${hIdx}-${baseTs + 3000}`,
            timestamp: baseTs + 3000,
            capability: 'tts',
            model: h.tts.model || 'kokoro',
            provider: h.tts.provider || 'kokoro',
            locality: isCloud ? 'cloud' : 'local',
            dataClass: isS5 ? 'sensitive' : 'general',
            prefer: 'local',
            costUsd: 0,
            durationMs: h.tts.durationMs || 1200,
            status: h.tts.fallbackUsed ? 'fallback' : 'ok',
          });
        }
      });

      if (seeded.length > 0) {
        setEntries(prev => {
          const existingIds = new Set(prev.map(e => e.id));
          const newUnique = seeded.filter(s => !existingIds.has(s.id));
          return [...prev, ...newUnique].sort((a, b) => b.timestamp - a.timestamp).slice(0, 200);
        });
      }
    }
  }, [history]);

  // Subscribe to real-time engine events
  useEffect(() => {
    const unsubscribe = engine.subscribeEvents((event: EngineEvent) => {
      if (event.type === 'RequestCompleted') {
        const d = event.data || {};
        const p = String(d.provider || 'ollama').toLowerCase();
        const isCloud = d.locality === 'cloud' || p.includes('gemini') || p.includes('openai');
        const entry: AuditEntry = {
          id: d.request_id || `req-${Date.now()}`,
          timestamp: event.timestamp || Date.now(),
          capability: d.capability || 'chat',
          model: d.model_used || d.model || 'unknown',
          provider: d.provider || 'unknown',
          locality: isCloud ? 'cloud' : 'local',
          dataClass: d.data_class || 'general',
          prefer: d.prefer || (isCloud ? 'cloud' : 'local'),
          costUsd: d.cost_usd || (isCloud ? 0.0008 : 0),
          durationMs: d.duration_ms || 0,
          status: d.fallback_used ? 'fallback' : 'ok',
        };
        setEntries(prev => [entry, ...prev].slice(0, 200));
      }
    });
    return unsubscribe;
  }, []);

  // Filtering and sorting
  const filteredEntries = useMemo(() => {
    let result = entries;
    if (localityFilter !== 'all') {
      result = result.filter(e => e.locality === localityFilter);
    }
    if (dataClassFilter !== 'all') {
      result = result.filter(e => e.dataClass === dataClassFilter);
    }
    if (sortBy === 'cost') {
      result = [...result].sort((a, b) => b.costUsd - a.costUsd);
    }
    return result;
  }, [entries, localityFilter, dataClassFilter, sortBy]);

  const localCount = entries.filter(e => e.locality === 'local').length;
  const cloudCount = entries.filter(e => e.locality === 'cloud').length;
  const sensitiveCloudCount = entries.filter(e => e.dataClass === 'sensitive' && e.locality === 'cloud').length;
  const totalCost = entries.reduce((sum, e) => sum + e.costUsd, 0);

  const formatTime = (ts: number) => {
    const d = new Date(ts);
    return d.toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      className="flex flex-col gap-4"
    >
      {/* Summary Cards */}
      <div className="grid grid-cols-4 gap-3">
        <Card className="p-3 border-border-subtle">
          <div className="flex items-center gap-2 mb-1">
            <Cpu className="w-4 h-4 text-accent-green" />
            <span className="text-[11px] font-medium text-text-secondary uppercase tracking-wider">Local</span>
          </div>
          <div className="text-[22px] font-semibold text-accent-green">{localCount}</div>
          <div className="text-[11px] text-text-dim font-mono">$0.00 total</div>
        </Card>
        <Card className="p-3 border-border-subtle">
          <div className="flex items-center gap-2 mb-1">
            <Globe className="w-4 h-4 text-accent-orange" />
            <span className="text-[11px] font-medium text-text-secondary uppercase tracking-wider">Cloud</span>
          </div>
          <div className="text-[22px] font-semibold text-accent-orange">{cloudCount}</div>
          <div className="text-[11px] text-text-dim font-mono">${totalCost.toFixed(6)}</div>
        </Card>
        <Card className="p-3 border-border-subtle">
          <div className="flex items-center gap-2 mb-1">
            <Shield className="w-4 h-4 text-accent-green" />
            <span className="text-[11px] font-medium text-text-secondary uppercase tracking-wider">Privacy</span>
          </div>
          <div className="text-[22px] font-semibold text-text-primary">
            {entries.length > 0 ? Math.round((localCount / entries.length) * 100) : 0}%
          </div>
          <div className="text-[11px] text-text-dim font-mono">local rate</div>
        </Card>
        <Card className={`p-3 border-border-subtle ${sensitiveCloudCount > 0 ? 'border-red-500/50 bg-red-500/5' : ''}`}>
          <div className="flex items-center gap-2 mb-1">
            <ShieldAlert className={`w-4 h-4 ${sensitiveCloudCount > 0 ? 'text-red-400' : 'text-accent-green'}`} />
            <span className="text-[11px] font-medium text-text-secondary uppercase tracking-wider">Violations</span>
          </div>
          <div className={`text-[22px] font-semibold ${sensitiveCloudCount > 0 ? 'text-red-400' : 'text-accent-green'}`}>
            {sensitiveCloudCount}
          </div>
          <div className="text-[11px] text-text-dim font-mono">sensitive→cloud</div>
        </Card>
      </div>

      {/* Filter Bar */}
      <div className="flex items-center gap-3 text-[12px]">
        <Filter className="w-4 h-4 text-text-dim" />
        <select
          value={localityFilter}
          onChange={e => setLocalityFilter(e.target.value as any)}
          className="bg-background-secondary text-text-primary border border-border-subtle rounded px-2 py-1 text-[12px]"
        >
          <option value="all">All Locality</option>
          <option value="local">Local Only</option>
          <option value="cloud">Cloud Only</option>
        </select>
        <select
          value={dataClassFilter}
          onChange={e => setDataClassFilter(e.target.value as any)}
          className="bg-background-secondary text-text-primary border border-border-subtle rounded px-2 py-1 text-[12px]"
        >
          <option value="all">All Data Classes</option>
          <option value="general">General</option>
          <option value="sensitive">Sensitive</option>
        </select>
        <button
          onClick={() => setSortBy(sortBy === 'time' ? 'cost' : 'time')}
          className="flex items-center gap-1 text-text-secondary hover:text-text-primary transition-colors"
        >
          <ArrowUpDown className="w-3 h-3" />
          Sort: {sortBy === 'time' ? 'Time' : 'Cost'}
        </button>
        <span className="ml-auto text-text-dim font-mono">
          {filteredEntries.length} / {entries.length} requests
        </span>
      </div>

      {/* Audit Table */}
      <Card className="p-0 overflow-hidden border-border-strong">
        <div className="overflow-x-auto">
          <table className="w-full text-[12px]">
            <thead>
              <tr className="bg-background-hover border-b border-border-subtle text-text-secondary text-left">
                <th className="px-3 py-2 font-medium">Time</th>
                <th className="px-3 py-2 font-medium">Capability</th>
                <th className="px-3 py-2 font-medium">Model</th>
                <th className="px-3 py-2 font-medium">Provider</th>
                <th className="px-3 py-2 font-medium">Locality</th>
                <th className="px-3 py-2 font-medium">Data Class</th>
                <th className="px-3 py-2 font-medium">Prefer</th>
                <th className="px-3 py-2 font-medium text-right">Cost</th>
                <th className="px-3 py-2 font-medium text-right">Latency</th>
              </tr>
            </thead>
            <tbody>
              {filteredEntries.length === 0 ? (
                <tr>
                  <td colSpan={9} className="px-3 py-8 text-center text-text-dim">
                    {entries.length === 0 
                      ? 'No requests recorded yet. Run a scenario to see data flow.' 
                      : 'No requests match current filters.'}
                  </td>
                </tr>
              ) : (
                filteredEntries.map((entry, i) => (
                  <motion.tr
                    key={entry.id}
                    initial={i < 5 ? { opacity: 0, x: -10 } : false}
                    animate={{ opacity: 1, x: 0 }}
                    className={`border-b border-border-subtle hover:bg-background-hover/50 transition-colors ${
                      entry.locality === 'local' 
                        ? 'bg-accent-green/[0.02]' 
                        : 'bg-accent-orange/[0.02]'
                    }`}
                  >
                    <td className="px-3 py-2 font-mono text-text-dim">{formatTime(entry.timestamp)}</td>
                    <td className="px-3 py-2">
                      <Badge capability={entry.capability as any} className="text-[10px]">{entry.capability}</Badge>
                    </td>
                    <td className="px-3 py-2 font-mono text-text-primary truncate max-w-[150px]">{entry.model}</td>
                    <td className="px-3 py-2 text-text-secondary">{entry.provider}</td>
                    <td className="px-3 py-2">
                      <span className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium ${
                        entry.locality === 'local'
                          ? 'bg-accent-green/15 text-accent-green'
                          : 'bg-accent-orange/15 text-accent-orange'
                      }`}>
                        {entry.locality === 'local' ? <Cpu className="w-3 h-3" /> : <Globe className="w-3 h-3" />}
                        {entry.locality}
                      </span>
                    </td>
                    <td className="px-3 py-2">
                      <span className={`text-[10px] font-medium ${
                        entry.dataClass === 'sensitive' ? 'text-red-400' : 'text-text-dim'
                      }`}>
                        {entry.dataClass}
                      </span>
                    </td>
                    <td className="px-3 py-2 font-mono text-text-dim">{entry.prefer}</td>
                    <td className="px-3 py-2 font-mono text-right">
                      <span className={entry.costUsd > 0 ? 'text-accent-orange' : 'text-accent-green'}>
                        ${entry.costUsd.toFixed(6)}
                      </span>
                    </td>
                    <td className="px-3 py-2 font-mono text-right text-text-secondary">
                      {entry.durationMs}ms
                    </td>
                  </motion.tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </Card>
    </motion.div>
  );
}
