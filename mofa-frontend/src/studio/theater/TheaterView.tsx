import React from 'react';
import { motion } from 'framer-motion';
import { motionVariants } from '../../lib/motion';
import { PipelinePhase } from '../usePipeline';
import { useElapsed } from '../useElapsed';
import { formatMs } from '../../lib/utils';
import { Card } from '../../shared/Card';
import { Check, AlertCircle, TerminalSquare, MessageSquareText, AudioLines, FileText, PlayCircle } from 'lucide-react';
import { getErrorDescriptor } from '../../errors/errorCatalog';
import { InlineError } from '../../errors/InlineError';
import { EventFeed } from '../../monitor/EventFeed';
import { PipelineViz } from '../PipelineViz';

interface TheaterViewProps {
  phase: PipelinePhase;
  onRetryTts: () => void;
  onReset: () => void;
}

export function TheaterView({ phase, onRetryTts, onReset }: TheaterViewProps) {
  const startedAt = (phase as any).startedAt;
  const elapsed = useElapsed(startedAt, phase.status !== 'error' && phase.status !== 'done');
  
  const isTranslating = phase.status === 'translating';
  const isSynthesizing = phase.status === 'synthesizing' || (phase.status === 'error' && phase.failedStep === 'tts');
  const isChatDone = phase.status === 'translated' || phase.status === 'synthesizing' || phase.status === 'done' || (phase.status === 'error' && phase.failedStep === 'tts');
  
  const chatResult = (phase as any).chat;
  const ttsResult = (phase as any).tts;

  return (
    <motion.div 
      className="flex-1 w-full flex flex-col overflow-hidden"
      variants={motionVariants.enter}
      initial="initial"
      animate="animate"
      exit="exit"
    >
      <div className="w-full max-w-5xl mx-auto px-6 py-8 flex flex-col flex-1 min-h-0">
        <div 
          className="flex justify-between items-center mb-8 shrink-0" 
        aria-live={phase.status === 'error' ? 'assertive' : 'polite'}
      >
        <h2 className="text-[20px] font-semibold text-text-primary">
          {phase.status === 'error' ? 'Generation failed' : isSynthesizing ? 'Synthesizing audio...' : 'Translating script...'}
        </h2>
        <div className="flex items-center gap-2 font-mono text-[15px]" aria-hidden="true">
          <span className="text-text-primary">{(elapsed / 1000).toFixed(1)}s</span>
          <span className="text-text-dim">/ ~{elapsed > 16000 ? '20' : '18'}s</span>
        </div>
      </div>

      {phase.status === 'error' && (
        <div className="mb-6 shrink-0">
          <InlineError 
            descriptor={getErrorDescriptor(phase.failedStep === 'chat' ? 'Chat' : 'Tts', phase.error.error)} 
            onAction={(actionCode) => {
              if (actionCode === 'retryTts') onRetryTts();
              else onReset();
            }} 
          />
        </div>
      )}

      {/* Zone B: Pipeline Viz */}
      <PipelineViz phase={phase} />

      {/* Zone C: Panels */}
      <div className="flex gap-6 flex-1 min-h-0">
        {/* Left: Events */}
        <Card className="w-[300px] flex flex-col min-h-0 bg-background-secondary border-black/5 shadow-sm p-0 overflow-hidden shrink-0">
          <div className="p-3 border-b border-black/5 flex items-center gap-2 bg-background-primary/50 shrink-0">
            <TerminalSquare className="w-4 h-4 text-accent-cyan" />
            <h3 className="text-[11px] font-semibold uppercase tracking-widest text-text-dim">Engine Events</h3>
          </div>
          <div className="flex-1 flex flex-col min-h-0 p-3">
            <EventFeed />
          </div>
        </Card>

        {/* Right: Script */}
        <Card className="flex-1 flex flex-col min-h-0 bg-white border-black/5 shadow-sm p-0 overflow-hidden">
           <div className="p-4 border-b border-black/5 flex items-center justify-between bg-background-secondary/50 shrink-0">
             <h3 className="text-[13px] font-medium text-text-primary tracking-wide">
               {isChatDone ? 'SCRIPT READY ✓' : 'TRANSLATING...'}
             </h3>
           </div>
           <div className="flex-1 p-6 overflow-y-auto">
             {!isChatDone ? (
               <div className="space-y-4 max-w-2xl mx-auto">
                 {[...Array(6)].map((_, i) => (
                   <div key={i} className={`h-4 bg-black/5 rounded animate-pulse w-${['3/4', 'full', '5/6', 'full', '2/3', '1/2'][i]}`} />
                 ))}
               </div>
             ) : (
               <motion.div 
                 initial={{ opacity: 0 }} animate={{ opacity: 1 }} 
                 className="text-[15px] leading-loose text-text-primary/90 font-sans max-w-2xl mx-auto"
               >
                 {chatResult?.script}
               </motion.div>
             )}
           </div>
        </Card>
      </div>
      </div>
    </motion.div>
  );
}
