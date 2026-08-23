import React from 'react';
import { motion } from 'framer-motion';
import { Check, AlertCircle, FileText, MessageSquareText, AudioLines, PlayCircle, Image, Video } from 'lucide-react';
import { formatMs } from '../lib/utils';
import { PipelinePhase } from './usePipeline';

export function PipelineViz({ phase }: { phase: PipelinePhase }) {
  const isTranslating = phase.status === 'translating';
  const isGeneratingImages = phase.status === 'generating_images';
  const isSynthesizing = phase.status === 'synthesizing';
  const isRenderingVideo = phase.status === 'rendering_video';
  const isChatDone = phase.status === 'translated' || isGeneratingImages || isSynthesizing || isRenderingVideo || phase.status === 'done' || (phase.status === 'error' && (phase as any).failedStep !== 'chat');
  
  const chatResult = (phase as any).chat;
  const imageResult = (phase as any).image;
  const ttsResult = (phase as any).tts;
  const videoResult = (phase as any).video;
  
  const scenarioId = (phase as any).scenarioId;
  const isVideo = scenarioId === 's4-explainer';

  return (
    <div className="w-full max-w-3xl mx-auto mb-8 relative mt-4 shrink-0">
      {/* Track */}
      <div className="absolute top-7 left-[70px] right-[70px] h-[3px] bg-background-hover rounded-full overflow-hidden z-0">
        <motion.div 
          className={`absolute inset-y-0 left-0 overflow-hidden ${phase.status === 'error' ? 'bg-accent-red' : phase.status === 'done' ? 'bg-accent-green' : (isSynthesizing || isRenderingVideo) ? 'bg-accent-purple' : 'bg-accent-blue'}`}
          initial={{ width: '0%' }}
          animate={{ 
            width: isVideo
              ? (phase.status === 'translating' ? '20%' : 
                 (phase.status === 'error' && (phase as any).failedStep === 'chat') ? '20%' : 
                 (phase.status === 'generating_images' || (phase.status === 'error' && (phase as any).failedStep === 'image')) ? '40%' : 
                 (phase.status === 'synthesizing' || (phase.status === 'error' && (phase as any).failedStep === 'tts')) ? '60%' : 
                 (phase.status === 'rendering_video' || (phase.status === 'error' && (phase as any).failedStep === 'video')) ? '80%' : 
                 phase.status === 'done' ? '100%' : '0%')
              : (phase.status === 'translating' ? '33.33%' : 
                 (phase.status === 'synthesizing' || (phase.status === 'error' && (phase as any).failedStep === 'tts')) ? '66.66%' : 
                 phase.status === 'done' ? '100%' : 
                 (phase.status === 'error' && (phase as any).failedStep === 'chat') ? '33.33%' : '0%') 
          }}
          transition={{ duration: 0.8, ease: "easeInOut" }}
        >
          {/* Active Flow Animation */}
          {(isTranslating || isGeneratingImages || isSynthesizing || isRenderingVideo) && (
            <motion.div 
              className="absolute inset-y-0 w-32 bg-gradient-to-r from-transparent via-white/50 to-transparent"
              initial={{ left: '-128px' }}
              animate={{ left: '100%' }}
              transition={{ duration: 1.5, repeat: Infinity, ease: 'linear' }}
            />
          )}
        </motion.div>
      </div>

      <div className="flex justify-between relative z-10 px-0">
        <PipelineNode 
          title="Input" 
          active={false} 
          done={true} 
          error={false} 
          accent="blue" 
          result={null}
          icon={FileText}
        />
        {isVideo ? (
          <>
            <PipelineNode 
              title="Script" 
              active={isTranslating} 
              done={isChatDone} 
              error={phase.status === 'error' && (phase as any).failedStep === 'chat'} 
              accent="blue" 
              result={chatResult}
              icon={MessageSquareText}
            />
            <PipelineNode 
              title="Image Gen" 
              active={phase.status === 'generating_images'} 
              done={['synthesizing', 'rendering_video', 'done'].includes(phase.status) || (phase.status === 'error' && !['chat', 'image'].includes((phase as any).failedStep))} 
              error={phase.status === 'error' && (phase as any).failedStep === 'image'} 
              accent="blue" 
              result={imageResult}
              icon={Image}
            />
            <PipelineNode 
              title="TTS Narration" 
              active={phase.status === 'synthesizing'} 
              done={['rendering_video', 'done'].includes(phase.status) || (phase.status === 'error' && (phase as any).failedStep === 'video')} 
              error={phase.status === 'error' && (phase as any).failedStep === 'tts'} 
              accent="purple" 
              result={ttsResult}
              icon={AudioLines}
            />
            <PipelineNode 
              title="Video Render" 
              active={phase.status === 'rendering_video'} 
              done={phase.status === 'done'} 
              error={phase.status === 'error' && (phase as any).failedStep === 'video'} 
              accent="purple" 
              result={videoResult}
              icon={Video}
            />
          </>
        ) : (
          <>
            <PipelineNode 
              title="Chat Generation" 
              active={isTranslating} 
              done={isChatDone} 
              error={phase.status === 'error' && (phase as any).failedStep === 'chat'} 
              accent="blue" 
              result={chatResult}
              icon={MessageSquareText}
            />
            <PipelineNode 
              title="TTS Audio" 
              active={isSynthesizing} 
              done={phase.status === 'done'} 
              error={phase.status === 'error' && (phase as any).failedStep === 'tts'} 
              accent="purple" 
              result={ttsResult}
              icon={AudioLines}
            />
          </>
        )}
        <PipelineNode 
          title="Ready" 
          active={false} 
          done={phase.status === 'done'} 
          error={false} 
          accent="green" 
          result={null}
          icon={PlayCircle}
        />
      </div>
    </div>
  );
}

export function PipelineNode({ 
  title, 
  active, 
  done, 
  error, 
  accent, 
  result, 
  icon: Icon 
}: { 
  title: string; 
  active: boolean; 
  done: boolean; 
  error: boolean; 
  accent: 'blue' | 'purple' | 'green'; 
  result: any;
  icon: any;
}) {
  let borderTextClass = "text-text-dim border-border-strong shadow-sm";
  let bgTintClass = "bg-transparent";
  
  if (active) {
    borderTextClass = accent === 'blue' 
      ? "text-accent-blue border-accent-blue/30 shadow-[0_0_20px_rgba(59,130,246,0.15)]" 
      : accent === 'purple' ? "text-accent-purple border-accent-purple/30 shadow-[0_0_20px_rgba(168,85,247,0.15)]"
      : "text-accent-green border-accent-green/30 shadow-[0_0_20px_rgba(16,185,129,0.15)]";
    bgTintClass = accent === 'blue' ? "bg-accent-blue/5" : accent === 'purple' ? "bg-accent-purple/5" : "bg-accent-green/5";
  } else if (error) {
    borderTextClass = "text-accent-red border-accent-red/30 shadow-sm";
    bgTintClass = "bg-accent-red/5";
  } else if (done) {
    borderTextClass = "text-accent-green border-accent-green/30 shadow-sm";
    bgTintClass = "bg-accent-green/5";
  }

  return (
    <div className="flex flex-col items-center gap-4 relative z-10 w-[140px]">
      <div className={`w-14 h-14 rounded-2xl border bg-background-card flex items-center justify-center transition-all duration-500 relative ${borderTextClass}`}>
        <div className={`absolute inset-0 rounded-2xl transition-colors duration-500 ${bgTintClass}`} />
        
        {/* Ring animation if active */}
        {active && (
           <motion.div 
             className="absolute inset-0 rounded-2xl border border-current opacity-50" 
             animate={{ scale: [1, 1.25, 1], opacity: [0.5, 0, 0.5] }} 
             transition={{ duration: 2, repeat: Infinity, ease: "easeInOut" }} 
           />
        )}
        
        {/* Status Badge */}
        {done && !error && (
          <motion.div 
            initial={{ scale: 0 }}
            animate={{ scale: 1 }}
            transition={{ type: 'spring', bounce: 0.5 }}
            className="absolute -top-1.5 -right-1.5 w-5 h-5 rounded-full bg-accent-green text-white flex items-center justify-center border-2 border-white shadow-sm z-10"
          >
            <Check className="w-3 h-3 stroke-[3]" />
          </motion.div>
        )}
        {error && (
          <motion.div 
            initial={{ scale: 0 }}
            animate={{ scale: 1 }}
            className="absolute -top-1.5 -right-1.5 w-5 h-5 rounded-full bg-accent-red text-white flex items-center justify-center border-2 border-white shadow-sm z-10"
          >
            <AlertCircle className="w-3 h-3 stroke-[3]" />
          </motion.div>
        )}
        
        <div className="relative flex items-center justify-center">
          {active && title === 'TTS Audio' ? (
            <div className="flex items-center gap-[2px] h-4">
              {[1, 2, 3, 4, 5].map(i => (
                <motion.div 
                  key={i} 
                  className="w-[2.5px] bg-current rounded-full origin-bottom" 
                  animate={{ scaleY: [0.3, 1, 0.3] }} 
                  transition={{ 
                    duration: 0.8, 
                    repeat: Infinity, 
                    ease: "easeInOut", 
                    delay: i * 0.1 
                  }} 
                  style={{ height: '100%' }} 
                />
              ))}
            </div>
          ) : (
            <Icon className={`w-6 h-6 ${active ? 'animate-pulse' : ''}`} />
          )}
        </div>
      </div>

      <div className="flex flex-col items-center text-center w-[160px]">
        <span className={`text-[12px] font-semibold tracking-wide uppercase ${active || done ? 'text-text-primary' : 'text-text-dim'}`}>
          {title}
        </span>
        <div className="mt-1.5 flex flex-col items-center gap-1 min-h-[32px]">
          {done && result && (
            <>
              <div className="flex items-center justify-center gap-1.5 flex-wrap">
                <span className="text-[10px] font-mono text-text-dim bg-background-hover px-2 py-0.5 rounded-full whitespace-nowrap">
                  {result.model ? `${result.model} · ` : ''}{formatMs(result.duration_ms ?? result.durationMs)}
                </span>
                {result.fallbackUsed && (
                  <span className="text-[10px] font-mono text-accent-yellow bg-accent-yellow/10 border border-accent-yellow/20 px-2 py-0.5 rounded-full whitespace-nowrap flex items-center gap-0.5">
                    ⚡ Fallback
                  </span>
                )}
              </div>
              {result.routingReason && (
                <span className="text-[9px] font-mono text-text-dim/60 mt-0.5 leading-[1.1] text-center max-w-[150px]">
                  Selected via: {result.routingReason.replace(/_/g, ' ')}
                </span>
              )}
            </>
          )}
          {active && (
            <span className={`text-[10px] font-mono font-medium animate-pulse ${accent === 'blue' ? 'text-accent-blue' : accent === 'purple' ? 'text-accent-purple' : 'text-accent-green'}`}>
              Processing...
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
