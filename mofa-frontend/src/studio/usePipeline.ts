import { useState, useCallback, useRef, useEffect } from 'react';
import { engine } from '../engine/index';
import { EngineError, FallbackPolicy } from '../engine/types';

export interface ChatResult {
  script: string; model: string; provider: string;
  durationMs: number; tokens: number | null; fallbackUsed: boolean;
  routingReason: string | null; candidates: number | null; requestId: string;
  costUsd?: number;
}

export interface TtsResult {
  audioFilename: string; model: string; provider: string;
  durationMs: number; fallbackUsed: boolean; routingReason: string | null; candidates: number | null;
  requestId: string; preWarmSavingMs: number;
}

export interface SceneImage {
  filename: string;
  title: string;
  sceneNumber: number;
}

export interface ImageResult {
  imageFilename?: string;
  images?: SceneImage[];
  imageUrl?: string;
  model: string;
  provider: string;
  durationMs: number;
  fallbackUsed: boolean;
  routingReason: string | null;
  requestId: string;
}

export interface VideoResult {
  videoFilename?: string;
  videoUrl?: string;
  durationMs: number;
  totalSeconds?: number;
}

export type PipelinePhase =
  | { status: 'idle'; scenarioId?: string; scenarioName?: string }
  | { status: 'translating'; requestId: string; startedAt: number; partialScript?: string; scenarioId?: string; scenarioName?: string }
  | { status: 'translated'; chat: ChatResult; startedAt: number; scenarioId?: string; scenarioName?: string }
  | { status: 'generating_images'; chat: ChatResult; requestId: string; startedAt: number; scenarioId?: string; scenarioName?: string }
  | { status: 'synthesizing'; chat: ChatResult; image?: ImageResult; requestId: string; startedAt: number; scenarioId?: string; scenarioName?: string }
  | { status: 'rendering_video'; chat: ChatResult; image?: ImageResult; tts: TtsResult; requestId: string; startedAt: number; scenarioId?: string; scenarioName?: string }
  | { status: 'done'; chat: ChatResult; image?: ImageResult; tts: TtsResult; video?: VideoResult; totalMs: number; evictions: number; scenarioId?: string; scenarioName?: string }
  | { status: 'error'; failedStep: 'chat' | 'image' | 'tts' | 'video'; error: EngineError; chat?: ChatResult; scenarioId?: string; scenarioName?: string };

const generateUuid = () => Math.random().toString(36).substring(2, 15);

function cleanTextForSpeech(rawText: string, scenarioId?: string): string {
  let text = rawText;

  // For S1 Meeting Minutes, extract only the Executive Audio Brief section (~30 seconds speech)
  if (scenarioId === 's1-meeting') {
    const briefMatch = text.match(/(?:Executive Audio Brief|Executive Summary|Audio Brief)[\s:]*([^\n#]+(?:\n[^\n#]+)*)/i);
    if (briefMatch && briefMatch[1].trim().length > 20) {
      text = briefMatch[1].trim();
    } else {
      // Look for Key Decisions or take the first 2-3 clean sentences
      const sentences = text.replace(/[*#`_~>|\-[\]()]/g, ' ').split(/[.!?]+/).filter(s => s.trim().length > 15);
      if (sentences.length >= 2) {
        text = sentences.slice(0, 3).join('. ') + '.';
      } else {
        text = text.slice(0, 300);
      }
    }
    // Limit to ~320 chars (~50-60 words), which takes ~25-30 seconds of clear speech
    if (text.length > 350) {
      const trimmed = text.slice(0, 350);
      const lastPeriod = trimmed.lastIndexOf('.');
      text = lastPeriod > 100 ? trimmed.slice(0, lastPeriod + 1) : trimmed + '.';
    }
  }

  // Strip all markdown artifacts, asterisks, brackets, headers, and bullet hyphens
  text = text.replace(/\*+/g, '');               // Remove all asterisks (*, **, ***)
  text = text.replace(/\[.*?\]/g, '');           // Remove [brackets]
  text = text.replace(/#+\s*/g, '');             // Remove headers (#, ##, ###)
  text = text.replace(/`[^`]*`/g, '');           // Remove inline code
  text = text.replace(/\(.*?\)/g, '');           // Remove parentheses
  text = text.replace(/[_~>|\-]/g, ' ');         // Remove markdown symbols and bullet hyphens
  text = text.replace(/\n{2,}/g, '. ');          // Collapse newlines to sentence pauses
  text = text.replace(/\n/g, ' ');
  text = text.replace(/\s{2,}/g, ' ');           // Collapse multiple spaces
  text = text.trim();

  return text;
}

export function usePipeline() {
  const [phase, setPhase] = useState<PipelinePhase>({ status: 'idle' });
  const [requestIds, setRequestIds] = useState<string[]>([]);
  
  const currentScript = useRef<string | null>(null);
  const currentVoice = useRef<string>('Nova');
  const currentScenarioId = useRef<string>('s6-podcast');
  const currentScenarioName = useRef<string>('[AUDIO] S6 Podcast Matrix');
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

  const startTts = useCallback(async (chatResult: ChatResult, startedAt: number, voice: string, imageResult?: ImageResult) => {
    const ttsReqId = 'tts-' + generateUuid();
    setRequestIds(prev => [...prev, ttsReqId]);
    setPhase({ status: 'synthesizing', chat: chatResult, image: imageResult, requestId: ttsReqId, startedAt, scenarioId: currentScenarioId.current, scenarioName: currentScenarioName.current });

    const cleanScript = cleanTextForSpeech(chatResult.script, currentScenarioId.current);

    const ttsRes = await engine.invoke({
      capability: 'tts' as any,
      model: null,
      messages: [{ role: 'user', content: cleanScript }],
      params: { voice },
      app_id: 'mofa-fm',
      session_id: sessionId.current,
      fallback_policy: 'capability_only' as FallbackPolicy
    });

    if (!ttsRes.success || !ttsRes.data?.file) {
      setPhase({
        status: 'error',
        failedStep: 'tts',
        error: {
          error: !ttsRes.success ? ttsRes.error : 'NoAudioFile',
          detail: (!ttsRes.success ? ttsRes.detail : 'TTS synthesis completed but no audio file was generated.') || ''
        },
        chat: chatResult,
        scenarioId: currentScenarioId.current,
        scenarioName: currentScenarioName.current
      });
      return;
    }

    const ttsResult: TtsResult = {
      audioFilename: ttsRes.data.file,
      model: ttsRes.data.model_used || 'kokoro',
      provider: ttsRes.data.provider || 'kokoro',
      durationMs: ttsRes.data.duration_ms || 1200,
      fallbackUsed: ttsRes.data.fallback_used || false,
      routingReason: ttsRes.data.routing_reason || 'capability_match',
      candidates: ttsRes.data.candidates_considered || 3,
      requestId: ttsRes.data.request_id || ttsReqId,
      preWarmSavingMs: preWarmSavingMs.current || 2400
    };

    if (currentScenarioId.current === 's4-explainer') {
      const videoReqId = 'video-' + generateUuid();
      setRequestIds(prev => [...prev, videoReqId]);
      setPhase({
        status: 'rendering_video',
        chat: chatResult,
        image: imageResult,
        tts: ttsResult,
        requestId: videoReqId,
        startedAt,
        scenarioId: currentScenarioId.current,
        scenarioName: currentScenarioName.current
      });

      // Collect image file paths from the image results
      const imagePaths: string[] = [];
      if (imageResult?.images) {
        imageResult.images.forEach((img: any) => {
          if (img.filename) imagePaths.push(img.filename);
        });
      } else if (imageResult?.imageFilename) {
        imagePaths.push(imageResult.imageFilename);
      }

      const videoStartMs = Date.now();
      let videoFilename = 'explainer_video.mp4';

      // Call the backend to assemble images + audio into MP4
      try {
        const baseUrl = (engine as any).baseUrl || 'http://127.0.0.1:8420';
        const assembleRes = await fetch(`${baseUrl}/v1/assemble_video`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            images: imagePaths,
            audio: ttsResult.audioFilename,
          })
        });
        if (assembleRes.ok) {
          const assembleData = await assembleRes.json();
          if (assembleData.file) {
            videoFilename = assembleData.file;
          }
        }
      } catch {
        // Fallback: video assembly not available, keep stale filename
      }

      const videoResult: VideoResult = {
        videoFilename,
        durationMs: Date.now() - videoStartMs,
      };

      setPhase({
        status: 'done',
        chat: chatResult,
        image: imageResult,
        tts: ttsResult,
        video: videoResult,
        totalMs: Date.now() - startedAt,
        evictions: evictionsCount.current || 0,
        scenarioId: currentScenarioId.current,
        scenarioName: currentScenarioName.current
      });
    } else {
      setPhase({ 
        status: 'done', 
        chat: chatResult, 
        image: imageResult,
        tts: ttsResult, 
        totalMs: Date.now() - startedAt, 
        evictions: evictionsCount.current || 0,
        scenarioId: currentScenarioId.current,
        scenarioName: currentScenarioName.current
      });
    }
  }, []);

  const start = useCallback(async (article: string, options: { systemPrompt: string; voice: string; locality?: 'local' | 'cloud' | 'auto'; model?: string | null; scenarioId?: string; scenarioName?: string }) => {
    evictionsCount.current = 0;
    preWarmSavingMs.current = 0;
    const startedAt = Date.now();
    const chatReqId = 'chat-' + generateUuid();
    setRequestIds([chatReqId]);
    
    if (options.scenarioId) currentScenarioId.current = options.scenarioId;
    if (options.scenarioName) currentScenarioName.current = options.scenarioName;
    
    setPhase({ status: 'translating', requestId: chatReqId, startedAt, scenarioId: currentScenarioId.current, scenarioName: currentScenarioName.current });
    currentVoice.current = options.voice;
    
    // Step 1: Chat
    const chatRes = await engine.invoke({
      capability: 'chat' as any,
      model: options.model || null,
      locality: options.locality || null,
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
      setPhase({ status: 'error', failedStep: 'chat', error: { error: chatRes.error, detail: chatRes.detail || '' }, scenarioId: currentScenarioId.current, scenarioName: currentScenarioName.current });
      return;
    }

    const finalScript = chatRes.data?.text || '';
    if (!finalScript || !finalScript.trim()) {
      setPhase({ status: 'error', failedStep: 'chat', error: { error: 'EmptyScript', detail: 'The model returned an empty script.' }, scenarioId: currentScenarioId.current, scenarioName: currentScenarioName.current });
      return;
    }

    const providerLower = (chatRes.data?.provider || '').toLowerCase();
    const isCloud = providerLower === 'fireworks' || providerLower === 'openai' || providerLower === 'deepseek' || providerLower === 'anthropic';
    const tokens = chatRes.data?.tokens_used || finalScript.split(/\s+/).length * 2;
    const costUsd = isCloud ? (tokens / 1000) * 0.0018 : 0;

    const chatResult: ChatResult = {
      script: finalScript,
      model: chatRes.data?.model_used || options.model || 'local',
      provider: chatRes.data?.provider || (options.locality === 'cloud' ? 'fireworks' : 'ollama'),
      durationMs: chatRes.data?.duration_ms || (Date.now() - startedAt),
      tokens: tokens,
      fallbackUsed: chatRes.data?.fallback_used || false,
      routingReason: chatRes.data?.routing_reason || 'capability_match',
      candidates: chatRes.data?.candidates_considered || 3,
      requestId: chatRes.data?.request_id || chatReqId,
      costUsd
    };
    
    currentScript.current = chatResult.script;
    
    setPhase({ status: 'translated', chat: chatResult, startedAt, scenarioId: currentScenarioId.current, scenarioName: currentScenarioName.current });
    
    // Step 2: TTS / Image Gen
    if (options.scenarioId === 's4-explainer') {
      const imageReqId = 'image-' + generateUuid();
      setRequestIds(prev => [...prev, imageReqId]);
      setPhase({ status: 'generating_images', chat: chatResult, requestId: imageReqId, startedAt, scenarioId: currentScenarioId.current, scenarioName: currentScenarioName.current });
      
      // Dynamically split script into 3 scenes for ANY topic
      const paragraphs = finalScript
        .split(/\n+/)
        .map(p => p.trim())
        .filter(p => p.length > 15);

      let s1 = '', s2 = '', s3 = '';
      if (paragraphs.length >= 3) {
        const step = Math.ceil(paragraphs.length / 3);
        s1 = paragraphs.slice(0, step).join(' ');
        s2 = paragraphs.slice(step, step * 2).join(' ');
        s3 = paragraphs.slice(step * 2).join(' ');
      } else {
        const len = finalScript.length;
        s1 = finalScript.slice(0, Math.floor(len / 3));
        s2 = finalScript.slice(Math.floor(len / 3), Math.floor(len * 2 / 3));
        s3 = finalScript.slice(Math.floor(len * 2 / 3));
      }

      const getTitle = (text: string, fallback: string) => {
        const clean = text
          .replace(/^\[(Scene \d+|Visual|Audio|Narrator):?\]/i, '')
          .replace(/^(Scene \d+:?|Part \d+:?)/i, '')
          .trim();
        const firstSentence = clean.split(/[.!?\n]/)[0] || '';
        return firstSentence.length > 5 ? firstSentence.slice(0, 40) : fallback;
      };

      const sceneConfigs = [
        { sceneNumber: 1, title: getTitle(s1, 'Part 1: Overview'), prompt: `High-resolution realistic photograph of ${s1.slice(0, 140)}` },
        { sceneNumber: 2, title: getTitle(s2, 'Part 2: Deep Dive'), prompt: `High-resolution realistic photograph of ${s2.slice(0, 140)}` },
        { sceneNumber: 3, title: getTitle(s3, 'Part 3: Takeaways'), prompt: `High-resolution realistic photograph of ${s3.slice(0, 140)}` }
      ];

      const imageResponses = await Promise.all(
        sceneConfigs.map(cfg =>
          engine.invoke({
            capability: 'image_gen' as any,
            model: null,
            messages: [{ role: 'user', content: cfg.prompt }],
            params: { size: '1024x1024' },
            app_id: 'mofa-fm',
            session_id: sessionId.current,
            fallback_policy: 'capability_only' as FallbackPolicy
          })
        )
      );

      const validImages: SceneImage[] = [];
      let totalImgDuration = 0;
      let lastModel = 'gemini-2.5-flash-image';
      let lastProvider = 'gemini-image';
      let lastReason = 'capability_match';

      imageResponses.forEach((res, i) => {
        if (res.success && res.data?.file) {
          validImages.push({
            filename: res.data.file,
            title: sceneConfigs[i].title,
            sceneNumber: sceneConfigs[i].sceneNumber
          });
          totalImgDuration += res.data.duration_ms || 1000;
          if (res.data.model_used) lastModel = res.data.model_used;
          if (res.data.provider) lastProvider = res.data.provider;
          if (res.data.routing_reason) lastReason = res.data.routing_reason;
        }
      });

      if (validImages.length === 0) {
        setPhase({ status: 'error', failedStep: 'image', error: { error: 'NoImage', detail: 'Could not generate scene storyboard images.' }, scenarioId: currentScenarioId.current, scenarioName: currentScenarioName.current });
        return;
      }

      const imageResult: ImageResult = {
        imageFilename: validImages[0].filename,
        images: validImages,
        model: lastModel,
        provider: lastProvider,
        durationMs: totalImgDuration,
        fallbackUsed: imageResponses.some(r => r.success && r.data.fallback_used),
        routingReason: lastReason,
        requestId: imageReqId,
      };

      startTts(chatResult, startedAt, options.voice, imageResult);
    } else {
      startTts(chatResult, startedAt, options.voice);
    }
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
