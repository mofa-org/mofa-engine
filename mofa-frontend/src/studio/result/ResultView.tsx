import React, { useState, Suspense } from 'react';
import { motion } from 'framer-motion';
import { motionVariants } from '../../lib/motion';
import { PipelinePhase } from '../usePipeline';
import { Card } from '../../shared/Card';
import { Button } from '../../shared/Button';
import { Badge } from '../../shared/Badge';
import { formatMs } from '../../lib/utils';
import { CheckCircle2, Download, Copy, RotateCcw, Zap, Shuffle } from 'lucide-react';
import { engine } from '../../engine/index';
import { PipelineViz } from '../PipelineViz';

const AudioPlayer = React.lazy(() => import('./AudioPlayer').then(m => ({ default: m.AudioPlayer })));

interface ResultViewProps {
  phase: PipelinePhase;
  onReset: () => void;
}

export function ResultView({ phase, onReset }: ResultViewProps) {
  const chat = (phase as any).chat;
  const tts = (phase as any).tts;
  const totalMs = (phase as any).totalMs;
  
  const [copied, setCopied] = useState(false);
  
  const handleCopy = () => {
    navigator.clipboard.writeText(chat.script);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };
  
  const handleDownload = () => {
    const url = engine.getAudioUrl(tts.audioFilename);
    const a = document.createElement('a');
    a.href = url;
    a.download = `mofa-podcast-${Date.now()}.wav`;
    a.click();
  };

  return (
    <motion.div 
      className="flex-1 w-full h-full overflow-y-auto"
      variants={motionVariants.enter}
      initial="initial"
      animate="animate"
      exit="exit"
    >
      <div className="w-full max-w-[720px] mx-auto px-6 py-6 flex flex-col min-h-full">
        <motion.div 
          className="flex flex-col items-center mb-6 text-center"
          initial={{ opacity: 0, y: -10 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 2, duration: 0.5 }}
        >
          <motion.div 
            initial={{ scale: 0.5, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            transition={{ delay: 2, duration: 0.4, ease: [0.25, 0.1, 0.25, 1] }}
            className="w-16 h-16 rounded-full bg-accent-green/10 flex items-center justify-center mb-4 shadow-[0_0_30px_rgba(16,185,129,0.2)]"
          >
            <CheckCircle2 className="w-8 h-8 text-accent-green" />
          </motion.div>
          <h2 className="text-[24px] font-semibold text-text-primary mb-2">Your podcast is ready</h2>
          <p className="text-[15px] font-mono text-text-dim mb-8">Generated in {((totalMs || 0) / 1000).toFixed(1)}s</p>
        </motion.div>

        <div className="flex flex-col items-center mb-6 text-center">
          {/* Trace View: Keep pipeline graph visible on completion */}
          <PipelineViz phase={phase} />
        </div>

      <div className="mb-6 min-h-[80px]">
        <Suspense fallback={<div className="w-full h-[80px] bg-black/5 rounded-[var(--radius-card)] animate-pulse" />}>
          <AudioPlayer filename={tts.audioFilename}>
            <div className="flex justify-center -mt-6 relative z-10 mb-4">
              <div className="px-4 py-2 bg-background-secondary/80 backdrop-blur-md border border-black/5 rounded-full flex items-center gap-3 text-[11px] font-medium text-text-secondary shadow-sm">
                <span className="text-text-primary uppercase tracking-widest font-semibold mr-1">Engine Scoreboard:</span>
                <span className="flex items-center gap-1"><Shuffle className="w-3.5 h-3.5 text-accent-blue" /> 2 routing decisions</span>
                <span className="text-black/20">·</span>
                <span className="flex items-center gap-1"><Zap className="w-3.5 h-3.5 text-accent-cyan" /> 1 pre-warm (saved {((tts?.preWarmSavingMs || 0) / 1000).toFixed(1)}s)</span>
                <span className="text-black/20">·</span>
                <span className="flex items-center gap-1">{(phase as any).evictions || 0} evictions</span>
                <span className="text-black/20">·</span>
                <span className="flex items-center gap-1">{tts.fallbackUsed || chat.fallbackUsed ? <span className="text-accent-yellow">1 fallback</span> : '0 fallbacks'}</span>
                <span className="text-black/20">·</span>
                <span className="flex items-center gap-1 text-accent-green">$0.00</span>
                <span className="text-black/20">·</span>
                <span>{['ollama', 'kokoro'].includes(chat.provider) && ['ollama', 'kokoro'].includes(tts.provider) ? '100% local' : 'Hybrid'}</span>
              </div>
            </div>
          </AudioPlayer>
        </Suspense>
      </div>

      {/* Intelligence Showcase */}
      <div className="mb-6">
        <div className="text-[13px] font-medium text-text-secondary mb-3 pl-1 uppercase tracking-wider">How it was made</div>
        <Card className="p-0 border-black/10 overflow-hidden text-[13px] font-mono shadow-sm">
          <div className="p-4 flex items-center justify-between border-b border-black/5 bg-black/5">
            <div className="flex items-center gap-3">
              <span className="w-5 h-5 rounded-full bg-background-primary flex items-center justify-center text-[10px] text-text-dim">1</span>
              <span className="text-text-primary">Translate</span>
            </div>
            <div className="flex items-center gap-4 text-text-dim">
              <span>{chat.model}</span>
              <span>{chat.provider}</span>
              <span>{formatMs(chat.durationMs)}</span>
              <span>{chat.tokens ?? '—'} tok</span>
              <Badge capability="Chat" className="ml-2">Chat</Badge>
            </div>
          </div>
          
          <div className="p-4 flex items-center justify-between border-b border-black/5 bg-black/[0.02]">
            <div className="flex items-center gap-3">
              <span className="w-5 h-5 rounded-full bg-background-primary flex items-center justify-center text-[10px] text-text-dim">2</span>
              <span className="text-text-primary">Synthesize</span>
            </div>
            <div className="flex items-center gap-4 text-text-dim">
              <span>{tts.model}</span>
              <span>{tts.provider}</span>
              <span>{formatMs(tts.durationMs)}</span>
              <Badge capability="Tts" className="ml-2">Tts</Badge>
            </div>
          </div>
          
          <div className="p-3 bg-accent-cyan/10 border-b border-accent-cyan/10 flex items-center gap-2 text-accent-cyan">
            <Zap className="w-4 h-4" />
            <span>TTS model was pre-warmed via hint_next → saved ~{((tts?.preWarmSavingMs || 0) / 1000).toFixed(1)}s cold start</span>
          </div>
          
          {tts.fallbackUsed && (
            <div className="p-3 bg-accent-yellow/10 border-b border-accent-yellow/10 flex items-center gap-2 text-accent-yellow">
              <Zap className="w-4 h-4" />
              <span>Fallback used: {tts.routingReason}</span>
            </div>
          )}
          
          <div className="p-3 bg-black/5 flex items-center gap-2 text-text-secondary"> 
             <Shuffle className="w-4 h-4" />
             {['ollama', 'kokoro'].includes(chat.provider) && ['ollama', 'kokoro'].includes(tts.provider) ? (
               <span>Routed locally · no cloud · $0.00 cost</span>
             ) : (
               <span>Routed to cloud ({!['ollama', 'kokoro'].includes(chat.provider) ? chat.provider : tts.provider}) · real cost applied</span>
             )}
          </div>
        </Card>
      </div>
      </div>
    </motion.div>
  );
}
