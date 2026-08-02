import React, { useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { ChevronDown, Brain, Sparkles } from 'lucide-react';
import { Badge } from '../../shared/Badge';
import { Card } from '../../shared/Card';

interface ThoughtChainViewProps {
  /** Array of reasoning token strings from the SSE stream */
  reasoningChunks: string[];
  /** Total reasoning token count */
  reasoningTokenCount: number;
  /** Whether reasoning is still streaming */
  isStreaming?: boolean;
}

/**
 * Collapsible thought chain display for reasoning API responses (PRD §S2).
 * 
 * Shows reasoning tokens in a muted, expandable section above the final output.
 * Designed for code review, contract analysis, and deep thinking workflows
 * where the user wants to optionally inspect the model's reasoning process.
 */
export function ThoughtChainView({ reasoningChunks, reasoningTokenCount, isStreaming = false }: ThoughtChainViewProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const reasoningText = reasoningChunks.join('');

  if (!reasoningText && !isStreaming) return null;

  return (
    <Card className="mb-4 p-0 overflow-hidden border-border-subtle bg-background-secondary/50">
      {/* Header — always visible */}
      <button
        onClick={() => setIsExpanded(!isExpanded)}
        className="w-full flex items-center justify-between px-4 py-3 hover:bg-background-hover transition-colors group"
      >
        <div className="flex items-center gap-3">
          <div className={`w-8 h-8 rounded-lg flex items-center justify-center ${
            isStreaming 
              ? 'bg-accent-purple/15 animate-pulse' 
              : 'bg-accent-purple/10'
          }`}>
            <Brain className="w-4 h-4 text-accent-purple" />
          </div>
          <div className="text-left">
            <div className="text-[13px] font-medium text-text-primary flex items-center gap-2">
              Reasoning Chain
              {isStreaming && (
                <motion.span
                  animate={{ opacity: [0.4, 1, 0.4] }}
                  transition={{ duration: 1.5, repeat: Infinity }}
                  className="text-[11px] text-accent-purple font-normal"
                >
                  thinking...
                </motion.span>
              )}
            </div>
            <div className="text-[11px] text-text-dim font-mono">
              {reasoningTokenCount} reasoning tokens
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Badge capability="Chat" className="text-[10px]">
            <Sparkles className="w-3 h-3 mr-1" />
            Deep Think
          </Badge>
          <motion.div
            animate={{ rotate: isExpanded ? 180 : 0 }}
            transition={{ duration: 0.2 }}
          >
            <ChevronDown className="w-4 h-4 text-text-dim group-hover:text-text-secondary transition-colors" />
          </motion.div>
        </div>
      </button>

      {/* Expandable reasoning content */}
      <AnimatePresence>
        {isExpanded && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.25, ease: [0.25, 0.1, 0.25, 1] }}
            className="overflow-hidden"
          >
            <div className="px-4 pb-4 border-t border-border-subtle">
              <pre className="mt-3 text-[12px] leading-relaxed text-text-dim font-mono whitespace-pre-wrap max-h-[300px] overflow-y-auto scrollbar-thin">
                {reasoningText}
                {isStreaming && (
                  <motion.span
                    animate={{ opacity: [0, 1] }}
                    transition={{ duration: 0.5, repeat: Infinity }}
                    className="inline-block w-2 h-4 bg-accent-purple/50 ml-0.5"
                  />
                )}
              </pre>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </Card>
  );
}
