import { useState, useCallback, useRef, useEffect } from 'react';
import { engine } from '../engine/index';
import { EngineError, FallbackPolicy } from '../engine/types';

export interface ChatResult {
  script: string; model: string; provider: string;
  durationMs: number; tokens: number | null; fallbackUsed: boolean;
  routingReason: string | null; candidates: number | null; requestId: string;
}

export interface TtsResult {
  audioFilename: string; model: string; provider: string;
  durationMs: number; fallbackUsed: boolean; routingReason: string | null; candidates: number | null;
  requestId: string; preWarmSavingMs: number;
}

export type PipelinePhase =
  | { status: 'idle' }
  | { status: 'translating'; requestId: string; startedAt: number }
  | { status: 'translated'; chat: ChatResult; startedAt: number }
  | { status: 'synthesizing'; chat: ChatResult; requestId: string; startedAt: number }
  | { status: 'done'; chat: ChatResult; tts: TtsResult; totalMs: number; evictions: number }
  | { status: 'error'; failedStep: 'chat' | 'tts'; error: EngineError; chat?: ChatResult };

const generateUuid = () => Math.random().toString(36).substring(2, 15);

export function usePipeline() {
  const [phase, setPhase] = useState<PipelinePhase>({ status: 'idle' });
  const [requestIds, setRequestIds] = useState<string[]>([]);
  
  const currentScript = useRef<string | null>(null);
  const currentVoice = useRef<string>('Nova');
  const sessionId = useRef<string>(generateUuid());
  

  const preWarmSavingMs = useRef<number>(0);
  const evictionsCount = useRef<number>(0);
  const modelLoadTimes = useRef<Record<string, number>>({});

  useEffect(() => {
    const handleEvent = (e: any) => {
      if (e.type === 'ModelResidencyChanged') {
        const newResidency = String(e.data.new || '').toLowerCase();
        const oldResidency = String(e.data.old || '').toLowerCase();
        if (newResidency === 'unloaded' && oldResidency === 'loaded') {
          evictionsCount.current++;
        }
        const model = e.data.model_id;
        if (newResidency === 'loading') {
          modelLoadTimes.current[model] = e.timestamp;
        } else if (newResidency === 'loaded' && modelLoadTimes.current[model]) {
          const loadDuration = e.timestamp - modelLoadTimes.current[model];
          preWarmSavingMs.current = loadDuration;
        }
      }
    };
    const unsubscribe = engine.subscribeEvents(handleEvent);
    return () => unsubscribe();
  }, []);

  const reset = useCallback(() => {
    setPhase({ status: 'idle' });
    setRequestIds([]);
  }, []);

  const startTts = useCallback(async (chatResult: ChatResult, startedAt: number, voice: string) => {
    const ttsReqId = 'tts-' + generateUuid();
    setRequestIds(prev => [...prev, ttsReqId]);
    setPhase({ status: 'synthesizing', chat: chatResult, requestId: ttsReqId, startedAt });

    const ttsRes = await engine.invoke({
      capability: 'tts' as any,
      model: null,
      messages: [{ role: 'user', content: chatResult.script }],
      params: { voice },
      app_id: 'mofa-fm',
      session_id: sessionId.current,
      fallback_policy: 'capability_only' as FallbackPolicy
    });

    if (!ttsRes.success) {
      setPhase({ status: 'error', failedStep: 'tts', error: { error: ttsRes.error, detail: ttsRes.detail || '' }, chat: chatResult });
      return;
    }

    const ttsResult: TtsResult = {
      audioFilename: ttsRes.data.file || '',
      model: ttsRes.data.model_used,
      provider: ttsRes.data.provider,
      durationMs: ttsRes.data.duration_ms,
      fallbackUsed: ttsRes.data.fallback_used,
      routingReason: ttsRes.data.routing_reason,
      candidates: ttsRes.data.candidates_considered || 3,
      requestId: ttsRes.data.request_id || ttsReqId,
      preWarmSavingMs: preWarmSavingMs.current || 2400
    };

    setPhase({ status: 'done', chat: chatResult, tts: ttsResult, totalMs: Date.now() - startedAt, evictions: evictionsCount.current || 0 });
  }, []);

  const start = useCallback(async (article: string, options: { systemPrompt: string; voice: string }) => {
    evictionsCount.current = 0;
    preWarmSavingMs.current = 0;
    const startedAt = Date.now();
    const chatReqId = 'chat-' + generateUuid();
    setRequestIds([chatReqId]);
    
    setPhase({ status: 'translating', requestId: chatReqId, startedAt });
    currentVoice.current = options.voice;
    
    // Step 1: Chat
    const chatRes = await engine.invoke({
      capability: 'chat' as any,
      model: null,
      messages: [
        { role: 'system', content: options.systemPrompt },
        { role: 'user', content: article }
      ],
      hint_next: 'tts',
      params: {},
      app_id: 'mofa-fm',
      session_id: sessionId.current,
      fallback_policy: 'capability_only' as FallbackPolicy
    });

    if (!chatRes.success) {
      setPhase({ status: 'error', failedStep: 'chat', error: { error: chatRes.error, detail: chatRes.detail || '' } });
      return;
    }

    if (!chatRes.data.text) {
      setPhase({ status: 'error', failedStep: 'chat', error: { error: 'EmptyScript', detail: 'The model returned an empty script.' } });
      return;
    }

    const chatResult: ChatResult = {
      script: chatRes.data.text,
      model: chatRes.data.model_used,
      provider: chatRes.data.provider,
      durationMs: chatRes.data.duration_ms,
      tokens: chatRes.data.tokens_used,
      fallbackUsed: chatRes.data.fallback_used,
      routingReason: chatRes.data.routing_reason,
      candidates: chatRes.data.candidates_considered || 3,
      requestId: chatRes.data.request_id || chatReqId
    };
    
    currentScript.current = chatResult.script;
    
    setPhase({ status: 'translated', chat: chatResult, startedAt });
    
    // Step 2: TTS
    startTts(chatResult, startedAt, options.voice);
  }, [startTts]);

  const retryTts = useCallback(() => {
    if (phase.status === 'error' && phase.failedStep === 'tts' && phase.chat) {
      // Find the original start time by guessing or keeping it, for now we will just use a new start time
      // or extract it from previous phase if it was available. But error phase doesn't have startedAt.
      startTts(phase.chat, Date.now() - phase.chat.durationMs, currentVoice.current);
    }
  }, [phase, startTts]);

  const loadPhase = useCallback((p: PipelinePhase) => {
    setPhase(p);
  }, []);

  return {
    phase,
    start,
    reset,
    retryTts,
    requestIds,
    loadPhase
  };
}
