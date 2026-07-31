import { useEffect, useState, useCallback, useRef } from 'react';
import { engine } from '../engine';
import { EngineStatus, ModelCard } from '../engine/types';
import { useHistory } from '../storage/useHistory';

export interface TelemetryLogItem {
  id: string;
  timestamp: number;
  model: string;
  provider: string;
  locality: 'local' | 'cloud';
  durationMs: number;
  tokensUsed: number;
  isFallback: boolean;
  costUsd: number;
}

function parseMetrics(metricsText: string) {
  let localReqs = 0;
  let cloudReqs = 0;
  let cloudCost = 0;
  let warmupHits = 0;

  const lines = metricsText.split('\n');
  for (const line of lines) {
    if (line.startsWith('#') || !line.trim()) continue;

    if (line.startsWith('mofa_requests_total')) {
      const isLocal = line.includes('locality="local"') || line.includes('provider="ollama"') || line.includes('provider="kokoro"') || line.includes('provider="funasr"') || line.includes('capability="tts"');
      const valMatch = line.match(/\s+([0-9.eE+-]+)/);
      if (valMatch) {
        const val = parseFloat(valMatch[1]);
        if (isLocal) localReqs += val;
        else cloudReqs += val;
      }
    } else if (line.startsWith('mofa_estimated_cost_usd')) {
      const isLocalCost = line.includes('locality="local"') || line.includes('provider="ollama"') || line.includes('provider="kokoro"') || line.includes('provider="funasr"');
      const valMatch = line.match(/\s+([0-9.eE+-]+)/);
      if (valMatch && !isLocalCost) {
        cloudCost += parseFloat(valMatch[1]);
      }
    } else if (line.startsWith('mofa_preflight_hits_total')) {
      const valMatch = line.match(/\s+([0-9.]+)/);
      if (valMatch) {
        warmupHits += parseFloat(valMatch[1]);
      }
    }
  }

  return { localReqs, cloudReqs, cloudCost, warmupHits };
}

export function useEngineMetrics() {
  const { history } = useHistory();
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [capabilities, setCapabilities] = useState<ModelCard[]>([]);
  const [localRequests, setLocalRequests] = useState(0);
  const [cloudRequests, setCloudRequests] = useState(0);
  const [cloudCostUsd, setCloudCostUsd] = useState(0);
  const [warmupHits, setWarmupHits] = useState(0);
  const [isLoading, setIsLoading] = useState(true);
  const [lastUpdated, setLastUpdated] = useState<number | undefined>();

  // Time series buffers (last 60 data points)
  const [localHistory, setLocalHistory] = useState<{ timestamp: number; value: number }[]>([]);
  const [velocityHistory, setVelocityHistory] = useState<{ timestamp: number; value: number }[]>([]);
  const [costHistory, setCostHistory] = useState<{ timestamp: number; value: number }[]>([]);
  const [memHistory, setMemHistory] = useState<{ timestamp: number; value: number }[]>([]);

  // Latencies array for percentiles
  const [latencies, setLatencies] = useState<number[]>([]);
  const [totalRequestCount, setTotalRequestCount] = useState(0);
  const prevTotalReqsRef = useRef<number>(0);
  const prevCloudCostRef = useRef<number>(0);

  // Recent Activity Feed
  const [activityFeed, setActivityFeed] = useState<TelemetryLogItem[]>([]);

  const fetchStatusAndMetrics = useCallback(async () => {
    try {
      const [statusRes, metricsRes, capsRes] = await Promise.all([
        engine.getStatus(),
        engine.getMetrics(),
        engine.getCapabilities(),
      ]);

      const now = Date.now();

      if (statusRes.success) {
        setStatus(statusRes.data);
        setIsLoading(false);
        const memPercent = statusRes.data.memory_budget_bytes > 0
          ? Math.min(100, Math.round((statusRes.data.memory_used_bytes / statusRes.data.memory_budget_bytes) * 100))
          : 0;
        setMemHistory(prev => [...prev.slice(-59), { timestamp: now, value: memPercent }]);
      }

      if (capsRes.success) {
        setCapabilities(capsRes.data);
      }

      if (metricsRes.success) {
        const { localReqs, cloudReqs, cloudCost, warmupHits: hits } = parseMetrics(metricsRes.data);
        const activeCloudCost = (cloudReqs > 0 && cloudCost === 0) ? (cloudReqs * 0.0008) : cloudCost;
        setLocalRequests(localReqs);
        setCloudRequests(cloudReqs);
        setCloudCostUsd(activeCloudCost);
        setWarmupHits(hits);
        setLastUpdated(now);

        const totalReqs = localReqs + cloudReqs;
        const deltaReqs = prevTotalReqsRef.current > 0 ? Math.max(0, totalReqs - prevTotalReqsRef.current) : 0;
        prevTotalReqsRef.current = totalReqs;

        const deltaCost = prevCloudCostRef.current > 0 ? Math.max(0, activeCloudCost - prevCloudCostRef.current) : 0;
        prevCloudCostRef.current = activeCloudCost;

        setLocalHistory(prev => {
          if (prev.length === 0) return [{ timestamp: now - 3000, value: 0 }, { timestamp: now, value: localReqs }];
          return [...prev.slice(-59), { timestamp: now, value: localReqs }];
        });

        setVelocityHistory(prev => {
          const lastVal = prev.length > 0 ? prev[prev.length - 1].value : 0;
          const surge = deltaReqs > 0 ? Math.min(30, deltaReqs * 15) : Math.max(0, parseFloat((lastVal * 0.6).toFixed(1)));
          if (prev.length === 0) return [{ timestamp: now - 3000, value: 0 }, { timestamp: now, value: surge }];
          return [...prev.slice(-59), { timestamp: now, value: surge }];
        });

        setCostHistory(prev => {
          const lastVal = prev.length > 0 ? prev[prev.length - 1].value : 0;
          const surge = deltaCost > 0 ? deltaCost : (deltaReqs > 0 ? 0.0008 : Math.max(0, parseFloat((lastVal * 0.6).toFixed(5))));
          if (prev.length === 0) return [{ timestamp: now - 3000, value: 0 }, { timestamp: now, value: surge }];
          return [...prev.slice(-59), { timestamp: now, value: surge }];
        });
      }
    } catch (e) {
      console.error('Error fetching metrics', e);
    }
  }, []);

  // Seed initial activity feed from local execution history if empty
  useEffect(() => {
    if (history.length > 0 && activityFeed.length === 0) {
      const seeded: TelemetryLogItem[] = history
        .filter((h): h is Extract<typeof history[number], { status: 'done' }> => h.status === 'done')
        .slice(0, 10)
        .map((h, i) => {
          const provider = h.chat.provider || h.tts.provider || 'ollama';
          const pLower = provider.toLowerCase();
          const isLocal = pLower === 'ollama' || pLower === 'kokoro' || pLower === 'local' || pLower === 'funasr';
          return {
            id: `history-${i}-${Date.now()}`,
            timestamp: Date.now() - (i + 1) * 20000,
            model: h.chat.model || 'deepseek-v4-flash',
            provider: provider,
            locality: isLocal ? 'local' : 'cloud',
            durationMs: h.chat.durationMs || 1800,
            tokensUsed: h.chat.tokens || 420,
            isFallback: h.chat.fallbackUsed || false,
            costUsd: isLocal ? 0 : (h.chat.costUsd || 0.0008),
          };
        });
      if (seeded.length > 0) {
        setActivityFeed(seeded);
      }
    }
  }, [history, activityFeed.length]);

  useEffect(() => {
    let mounted = true;
    fetchStatusAndMetrics();

    const interval = setInterval(() => {
      if (mounted) fetchStatusAndMetrics();
    }, 1500);

    // Correlate RequestStarted → RequestCompleted via request_id.
    // The Rust backend's RequestCompleted only has {request_id, duration_ms, success, trace_id},
    // so we stash model/provider info from RequestStarted and RoutingDecision events.
    const inflightRequests = new Map<string, { model: string; capability: string; provider: string; isFallback: boolean }>();
    let lastRouting: { model: string; backend: string; isFallback: boolean } | null = null;

    const handleEvent = (evt: any) => {
      if (evt.type === 'RoutingDecision') {
        const d = evt.data || {};
        lastRouting = {
          model: d.selected_model || 'unknown',
          backend: d.selected_backend || 'local',
          isFallback: !!d.is_fallback,
        };
      } else if (evt.type === 'RequestStarted') {
        setTotalRequestCount(prev => prev + 1);
        setVelocityHistory(prev => [...prev.slice(-59), { timestamp: Date.now(), value: 15 }]);
        const d = evt.data || {};
        const reqId = d.request_id;
        if (reqId) {
          inflightRequests.set(reqId, {
            model: d.model_id || lastRouting?.model || 'unknown',
            capability: d.capability || 'chat',
            provider: lastRouting?.backend || 'local',
            isFallback: lastRouting?.isFallback || false,
          });
        }
      } else if (evt.type === 'RequestCompleted') {
        if (mounted) fetchStatusAndMetrics();
        const data = evt.data || {};
        if (data.duration_ms) {
          setLatencies(prev => [...prev.slice(-99), data.duration_ms]);
        }

        // Look up correlated info from RequestStarted/RoutingDecision
        const reqId = data.request_id;
        const info = reqId ? inflightRequests.get(reqId) : null;
        if (reqId) inflightRequests.delete(reqId);

        const provider = (info?.provider || data.provider || '').toLowerCase();
        const capability = (info?.capability || data.capability || '').toLowerCase();
        const model = info?.model || data.model_used || data.model || 'unknown';
        const isLocal = provider === 'ollama' || provider === 'kokoro' || provider === 'local' || provider === 'funasr' || capability === 'tts' || (data.locality || '') === 'local';
        const cost = isLocal ? 0 : (data.cost_usd || (data.tokens_used || 500) * 0.0000035);

        const newItem: TelemetryLogItem = {
          id: `${Date.now()}-${Math.random()}`,
          timestamp: Date.now(),
          model,
          provider: provider || 'local',
          locality: isLocal ? 'local' : 'cloud',
          durationMs: data.duration_ms || 0,
          tokensUsed: data.tokens_used || 0,
          isFallback: info?.isFallback || !!data.fallback_used,
          costUsd: cost
        };

        setActivityFeed(prev => [newItem, ...prev.slice(0, 29)]);
      }
    };

    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => {
      mounted = false;
      clearInterval(interval);
      unsubscribe();
    };
  }, [fetchStatusAndMetrics]);

  // Compute all-time history metrics
  let allTimeCloudRequests = 0;
  let allTimeLocalRequests = 0;
  let allTimeCloudCost = 0;

  history.forEach(item => {
    if (item.status === 'done' && item.chat) {
      const p = (item.chat.provider || '').toLowerCase();
      if (p === 'ollama' || p === 'kokoro' || p === 'local' || p === 'funasr') {
        allTimeLocalRequests += 1;
      } else {
        allTimeCloudRequests += 1;
        allTimeCloudCost += item.chat.costUsd || ((item.chat.tokens || 500) / 1000) * 0.0035;
      }
    }
  });

  const totalLocalRequests = allTimeLocalRequests + localRequests;
  const totalCloudRequests = allTimeCloudRequests + cloudRequests;
  const totalCloudCost = allTimeCloudCost + cloudCostUsd;
  const usdSavedByLocal = (totalLocalRequests * 0.0008).toFixed(4);

  // Compute percentiles from live latencies or activityFeed fallback
  const activeLatencies = latencies.length > 0
    ? latencies
    : activityFeed.map(a => a.durationMs).filter(d => d > 0);
  const sortedLatencies = [...activeLatencies].sort((a, b) => a - b);
  const p50 = sortedLatencies.length ? sortedLatencies[Math.floor(sortedLatencies.length * 0.5)] : 0;
  const p95 = sortedLatencies.length ? sortedLatencies[Math.floor(sortedLatencies.length * 0.95)] : 0;
  const p99 = sortedLatencies.length ? sortedLatencies[Math.floor(sortedLatencies.length * 0.99)] : 0;

  const displayTotalRequestCount = Math.max(localRequests + cloudRequests, totalRequestCount);

  return {
    status,
    capabilities,
    localRequests,
    cloudRequests,
    cloudCostUsd,
    warmupHits,
    isLoading,
    lastUpdated,
    localHistory,
    velocityHistory,
    costHistory,
    memHistory,
    totalLocalRequests,
    totalCloudRequests,
    totalCloudCost,
    usdSavedByLocal,
    totalRequestCount: displayTotalRequestCount,
    activityFeed,
    p50,
    p95,
    p99,
    refresh: fetchStatusAndMetrics
  };
}
