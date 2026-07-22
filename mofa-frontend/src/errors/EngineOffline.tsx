import React, { useState } from 'react';
import { motion } from 'framer-motion';
import { Card } from '../shared/Card';
import { Button } from '../shared/Button';
import { RefreshCw, ZapOff, CheckCircle2, Copy } from 'lucide-react';
import { useEngineUrl } from '../engine';

export function EngineOffline() {
  const url = useEngineUrl();
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText('cargo run --bin mofa-engine');
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <motion.div 
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0, scale: 0.95 }}
      transition={{ duration: 0.3, ease: [0.25, 0.1, 0.25, 1] }}
      className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-background-primary/80 backdrop-blur-md"
    >
      <Card className="max-w-md w-full p-8 text-center flex flex-col items-center bg-black/40 border-white/10 shadow-[0_0_40px_rgba(0,0,0,0.5)]">
        <div className="w-16 h-16 rounded-full bg-accent-red/10 flex items-center justify-center mb-6 shadow-[0_0_20px_rgba(239,68,68,0.2)]">
          <ZapOff className="w-8 h-8 text-accent-red" />
        </div>
        
        <h2 className="text-xl font-medium text-text-primary mb-2">Can't reach the MoFA engine</h2>
        <p className="text-sm text-text-secondary mb-8">
          The engine at <span className="font-mono text-accent-cyan">{url}</span> is offline.
        </p>
        
        <div className="w-full bg-black/40 border border-white/5 rounded-md p-4 flex flex-col items-start group mb-8">
          <div className="text-[11px] font-semibold uppercase tracking-widest text-text-dim mb-2">Start the engine</div>
          <div className="flex justify-between items-center w-full">
            <code className="text-sm font-mono text-white select-all">
              cargo run --bin mofa-engine
            </code>
            <button 
              onClick={handleCopy}
              className="p-1.5 rounded-sm hover:bg-white/10 text-text-dim hover:text-white transition-colors"
            >
              {copied ? <CheckCircle2 className="w-4 h-4 text-accent-green" /> : <Copy className="w-4 h-4" />}
            </button>
          </div>
        </div>

        <div className="flex gap-4 w-full">
          <Button variant="primary" className="flex-1 gap-2" onClick={() => window.location.reload()}>
            <RefreshCw className="w-4 h-4" />
            Retry now
          </Button>
        </div>
      </Card>
    </motion.div>
  );
}
