import React, { useEffect } from 'react';
import { AnimatePresence } from 'framer-motion';
import { usePipeline } from './usePipeline';
import { ComposeView } from './compose/ComposeView';
import { TheaterView } from './theater/TheaterView';
import { ResultView } from './result/ResultView';
import { useHistory } from '../storage/useHistory';
import { useSettings } from '../storage/useSettings';
import { playChime } from '../lib/audio';

export function Studio() {
  const { phase, start, reset, retryTts, loadPhase } = usePipeline();
  const { saveToHistory } = useHistory();
  const { settings } = useSettings();

  useEffect(() => {
    if (phase.status === 'done') {
      saveToHistory(phase);
      if (settings.soundEnabled && !settings.reducedMotion) {
        playChime();
      }
    }
  }, [phase, saveToHistory, settings.soundEnabled, settings.reducedMotion]);

  useEffect(() => {
    const handleLoad = (e: any) => loadPhase(e.detail);
    document.addEventListener('load-history-phase', handleLoad);
    return () => document.removeEventListener('load-history-phase', handleLoad);
  }, [loadPhase]);

  useEffect(() => {
    if (phase.status === 'idle') {
      document.title = 'MoFA FM — Local AI Podcast Studio';
    } else if (phase.status === 'done') {
      document.title = '✓ Ready — MoFA FM';
    } else if (phase.status === 'error') {
      document.title = 'Error — MoFA FM';
    } else {
      document.title = '⚡ Generating... — MoFA FM';
    }
  }, [phase.status]);

  return (
    <main className="flex-1 w-full flex flex-col relative overflow-hidden">
      <AnimatePresence mode="wait">
        {phase.status === 'idle' && (
          <ComposeView key="compose" onStart={start} />
        )}
        
        {(phase.status === 'translating' || phase.status === 'translated' || phase.status === 'synthesizing' || phase.status === 'error') && (
          <TheaterView key="theater" phase={phase} onRetryTts={retryTts} onReset={reset} />
        )}
        
        {phase.status === 'done' && (
          <ResultView key="result" phase={phase} onReset={reset} />
        )}
      </AnimatePresence>
    </main>
  );
}
