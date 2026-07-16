import React, { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { X, History, Trash2, Play } from 'lucide-react';
import { useHistory } from './useHistory';
import { PipelinePhase } from '../studio/usePipeline';
import { formatMs } from '../lib/utils';
import { Badge } from '../shared/Badge';

interface HistoryDrawerProps {
  onSelectResult?: (phase: PipelinePhase) => void;
}

export function HistoryDrawer({ onSelectResult }: HistoryDrawerProps) {
  const [isOpen, setIsOpen] = useState(false);
  const { history, clearHistory } = useHistory();

  useEffect(() => {
    const handleOpen = () => setIsOpen(true);
    document.addEventListener('open-history', handleOpen);
    return () => document.removeEventListener('open-history', handleOpen);
  }, []);

  const handleSelect = (phase: PipelinePhase) => {
    setIsOpen(false);
    if (onSelectResult) {
      onSelectResult(phase);
    } else {
      document.dispatchEvent(new CustomEvent('load-history-phase', { detail: phase }));
    }
  };

  return (
    <AnimatePresence>
      {isOpen && (
        <>
          <motion.div 
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={() => setIsOpen(false)}
            className="fixed inset-0 bg-black/40 backdrop-blur-sm z-40"
          />
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-label="History"
            initial={{ x: '-100%' }}
            animate={{ x: 0 }}
            exit={{ x: '-100%' }}
            transition={{ duration: 0.3, ease: [0.25, 0.1, 0.25, 1] }}
            className="fixed left-0 top-0 bottom-0 w-[360px] bg-background-secondary/90 backdrop-blur-xl border-r border-black/10 z-50 flex flex-col shadow-2xl"
          >
            <div className="h-16 border-b border-black/5 flex items-center justify-between px-6 shrink-0 bg-background-primary/50">
              <div className="flex items-center gap-2 text-text-primary">
                <History className="w-5 h-5 text-accent-purple" />
                <h2 className="font-medium text-[15px]">History</h2>
              </div>
              <button 
                onClick={() => setIsOpen(false)}
                className="p-2 -mr-2 text-text-secondary hover:text-black rounded-full hover:bg-black/5 transition-colors"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
            
            <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-3">
              {history.length === 0 ? (
                <div className="text-text-dim text-center mt-10 text-[13px]">
                  No past generations.
                </div>
              ) : (
                <>
                  <div className="flex justify-end mb-2">
                    <button 
                      onClick={clearHistory}
                      className="text-[11px] flex items-center gap-1 text-text-dim hover:text-accent-red transition-colors"
                    >
                      <Trash2 className="w-3.5 h-3.5" /> Clear All
                    </button>
                  </div>
                  {history.map((item, idx) => (
                    item.status === 'done' && (
                      <div 
                        key={idx}
                        onClick={() => handleSelect(item)}
                        className="p-4 bg-white border border-black/5 shadow-sm rounded-md cursor-pointer hover:bg-black/5 hover:border-black/10 transition-all group"
                      >
                        <div className="flex justify-between items-start mb-2">
                          <div className="text-[13px] font-medium text-text-primary line-clamp-1">
                            {item.chat.script.substring(0, 40)}...
                          </div>
                          <Play className="w-4 h-4 text-accent-cyan opacity-0 group-hover:opacity-100 transition-opacity" />
                        </div>
                        
                        <div className="flex items-center gap-2 mt-3 flex-wrap">
                          <Badge capability="Chat" className="scale-90 origin-left">{item.chat.model}</Badge>
                          <Badge capability="Tts" className="scale-90 origin-left">{item.tts.model}</Badge>
                        </div>
                        <div className="text-[10px] font-mono text-text-dim mt-2 pt-2 border-t border-black/5 flex justify-between">
                          <span>{formatMs(item.totalMs)}</span>
                          <span>{item.chat.tokens} tokens</span>
                        </div>
                      </div>
                    )
                  ))}
                </>
              )}
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
