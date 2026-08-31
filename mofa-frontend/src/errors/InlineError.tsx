import { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { Card } from '../shared/Card';
import { Button } from '../shared/Button';
import { XCircle, Copy, CheckCircle2, RotateCcw } from 'lucide-react';
import { ErrorDescriptor } from './errorCatalog';

interface InlineErrorProps {
  descriptor: ErrorDescriptor;
  onAction: (actionCode: string) => void;
}

export function InlineError({ descriptor, onAction }: InlineErrorProps) {
  const [copied, setCopied] = useState(false);
  const [countdown, setCountdown] = useState<number | null>(
    descriptor.autoRecoverMs ? Math.ceil(descriptor.autoRecoverMs / 1000) : null
  );

  useEffect(() => {
    if (descriptor.autoRecoverMs) {
      const interval = setInterval(() => {
        setCountdown((c) => {
          if (c === null || c <= 1) {
            clearInterval(interval);
            onAction(descriptor.actionCode);
            return null;
          }
          return c - 1;
        });
      }, 1000);
      return () => clearInterval(interval);
    }
  }, [descriptor, onAction]);

  const handleCopy = () => {
    if (descriptor.snippet) {
      navigator.clipboard.writeText(descriptor.snippet);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: 10, scale: 0.95 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={{ duration: 0.3, ease: [0.25, 0.1, 0.25, 1] }}
      className="mb-6 shrink-0"
    >
      <Card className="p-5 border-accent-red/20 bg-accent-red/5 relative overflow-hidden">
        <div className="absolute top-0 left-0 w-1 h-full bg-accent-red/50" />
        <div className="flex items-start gap-4">
          <div className="mt-1">
            <XCircle className="w-5 h-5 text-accent-red/80" />
          </div>
          <div className="flex-1">
            <h4 className="text-[15px] font-semibold text-white mb-1">
              {descriptor.title || 'Error'}
            </h4>
            <p className="text-sm text-text-dim mb-4 leading-relaxed">
              {descriptor.message}
            </p>
            
            {descriptor.snippet && (
              <div className="mb-4 bg-background-secondary border border-border-subtle rounded-md p-3 flex justify-between items-center group">
                <code className="text-[13px] font-mono text-accent-cyan/80 select-all">
                  {descriptor.snippet}
                </code>
                <button 
                  onClick={handleCopy}
                  className="p-1.5 rounded-sm hover:bg-white/10 text-text-dim hover:text-white transition-colors"
                >
                  {copied ? <CheckCircle2 className="w-4 h-4 text-accent-green" /> : <Copy className="w-4 h-4" />}
                </button>
              </div>
            )}

            <div className="flex items-center gap-3">
              <Button 
                variant="secondary" 
                onClick={() => onAction(descriptor.actionCode)}
                className="bg-white/5 hover:bg-white/10 border-border-strong"
              >
                {descriptor.actionLabel}
              </Button>
              {countdown !== null && (
                <span className="text-[13px] font-mono text-text-dim flex items-center gap-2">
                  <RotateCcw className="w-3.5 h-3.5 animate-spin-slow" />
                  Retrying in {countdown}s...
                </span>
              )}
            </div>
          </div>
        </div>
      </Card>
    </motion.div>
  );
}
