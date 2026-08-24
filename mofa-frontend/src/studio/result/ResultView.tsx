import React, { Suspense } from 'react';
import { motion } from 'framer-motion';
import { motionVariants } from '../../lib/motion';
import { PipelinePhase } from '../usePipeline';
import { Card } from '../../shared/Card';
import { Badge } from '../../shared/Badge';
import { formatMs } from '../../lib/utils';
import { CheckCircle2, Zap, Shuffle, Video, FolderOpen, ArrowRight } from 'lucide-react';
import { PipelineViz } from '../PipelineViz';
import { ThoughtChainView } from './ThoughtChainView';

const AudioPlayer = React.lazy(() => import('./AudioPlayer').then(m => ({ default: m.AudioPlayer })));

interface ResultViewProps {
  phase: PipelinePhase;
  onReset: () => void;
}

export function ResultView({ phase, onReset: _onReset }: ResultViewProps) {
  const chat = (phase as any).chat;
  const tts = (phase as any).tts;
  const totalMs = (phase as any).totalMs;
  const scenarioId = (phase as any).scenarioId || 's6-podcast';
  const isVideo = scenarioId === 's4-explainer';

  const title = isVideo
    ? 'Your explainer video narration is ready'
    : scenarioId === 's2-review'
    ? 'Your code review is ready'
    : scenarioId === 's1-meeting'
    ? 'Your meeting brief is ready'
    : 'Your podcast is ready';

  const reasoningChunks = chat?.reasoningChunks || (chat?.reasoningText ? [chat.reasoningText] : []);
  const reasoningTokenCount = chat?.reasoningTokenCount || (reasoningChunks.join('').split(/\s+/).length);

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
          <h2 className="text-[24px] font-semibold text-text-primary mb-2">{title}</h2>
          <p className="text-[15px] font-mono text-text-dim mb-8">Generated in {((totalMs || 0) / 1000).toFixed(1)}s</p>
        </motion.div>

        {isVideo && (
          <div className="mb-6 space-y-4">
            {/* Embedded Video Player */}
            {(phase as any).video?.videoFilename && (
              <div className="rounded-2xl overflow-hidden border border-purple-500/30 shadow-lg bg-black">
                <video
                  controls
                  autoPlay
                  className="w-full aspect-video"
                  src={`http://127.0.0.1:8420/v1/files/${(phase as any).video.videoFilename.split('/').pop()}`}
                >
                  Your browser does not support the video tag.
                </video>
                <div className="p-3 bg-purple-500/10 flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <Video className="w-4 h-4 text-purple-400" />
                    <span className="text-xs font-semibold text-text-primary">Explainer Video ({((phase as any).video?.durationMs || 0) / 1000}s render)</span>
                  </div>
                  <a
                    href={`http://127.0.0.1:8420/v1/files/${(phase as any).video.videoFilename.split('/').pop()}`}
                    download
                    className="flex items-center gap-1.5 px-3 py-1.5 bg-purple-500 text-white rounded-lg text-xs font-medium hover:bg-purple-600 transition-colors shadow-sm"
                  >
                    Download MP4 <ArrowRight className="w-3.5 h-3.5" />
                  </a>
                </div>
              </div>
            )}

            {/* 3-Scene Visual Storyboard Grid */}
            {((phase as any).image?.images || ((phase as any).image?.imageFilename ? [{ filename: (phase as any).image.imageFilename, title: 'Scene 1: Foundation', sceneNumber: 1 }] : [])).length > 0 && (
              <div className="p-4 bg-background-secondary/80 border border-border-subtle rounded-2xl">
                <div className="flex items-center justify-between mb-3 px-1">
                  <span className="text-[12px] font-semibold uppercase tracking-wider text-text-secondary flex items-center gap-1.5">
                    [STUDIO] Visual Storyboard ({((phase as any).image?.images?.length || 1)} Scenes)
                  </span>
                </div>
                <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                  {((phase as any).image?.images || [{ filename: (phase as any).image.imageFilename, title: 'Scene 1: Foundation', sceneNumber: 1 }]).map((img: any, idx: number) => (
                    <a 
                      key={idx} 
                      href={`http://127.0.0.1:8420/v1/files/${img.filename.split('/').pop()}`}
                      target="_blank"
                      rel="noreferrer"
                      className="group aspect-video sm:aspect-square bg-background-hover border border-border-subtle rounded-xl overflow-hidden relative shadow-sm hover:border-purple-500/50 transition-all cursor-pointer block"
                    >
                      <img 
                        src={`http://127.0.0.1:8420/v1/files/${img.filename.split('/').pop()}`} 
                        className="w-full h-full object-cover group-hover:scale-105 transition-transform duration-300" 
                        alt={img.title} 
                      />
                      <div className="absolute top-2 left-2 px-1.5 py-0.5 bg-black/70 backdrop-blur-md rounded text-[9px] text-white font-mono border border-white/10">
                        Scene {img.sceneNumber || (idx + 1)}/3
                      </div>
                      <div className="absolute inset-x-0 bottom-0 p-2 bg-gradient-to-t from-black/80 via-black/40 to-transparent">
                        <p className="text-[10px] text-white font-medium truncate">{img.title}</p>
                      </div>
                    </a>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}


        <div className="flex flex-col items-center mb-6 text-center">
          {/* Trace View: Keep pipeline graph visible on completion */}
          <PipelineViz phase={phase} />
        </div>

      <div className="mb-6 min-h-[80px]">
        <Suspense fallback={<div className="w-full h-[80px] bg-background-hover rounded-[var(--radius-card)] animate-pulse" />}>
          <AudioPlayer filename={tts.audioFilename}>
            <div className="flex justify-center -mt-6 relative z-10 mb-4">
              <div className="px-4 py-2 bg-background-secondary/80 backdrop-blur-md border border-border-subtle rounded-full flex items-center gap-3 text-[11px] font-medium text-text-secondary shadow-sm">
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

      {/* Thought Chain Display for Deep Thinking Reasoning */}
      {reasoningChunks.length > 0 && (
        <div className="mb-6">
          <ThoughtChainView
            reasoningChunks={reasoningChunks}
            reasoningTokenCount={reasoningTokenCount}
            isStreaming={false}
          />
        </div>
      )}

      {/* Intelligence Showcase */}
      <div className="mb-6">
        <div className="text-[13px] font-medium text-text-secondary mb-3 pl-1 uppercase tracking-wider">How it was made</div>
        <Card className="p-0 border-border-strong overflow-hidden text-[13px] font-mono shadow-sm">
          <div className="p-4 flex items-center justify-between border-b border-border-subtle bg-background-hover">
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
          
          <div className="p-4 flex items-center justify-between border-b border-border-subtle bg-background-hover">
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
          
          <div className="p-3 bg-background-hover flex items-center gap-2 text-text-secondary"> 
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
