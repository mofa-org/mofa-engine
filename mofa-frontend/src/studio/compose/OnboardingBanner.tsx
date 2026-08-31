import React, { useState } from 'react';
import { Info, X } from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';

export function OnboardingBanner() {
  const [show, setShow] = useState(() => {
    return localStorage.getItem('mofa_onboarding_dismissed') !== 'true';
  });

  const handleDismiss = () => {
    setShow(false);
    localStorage.setItem('mofa_onboarding_dismissed', 'true');
  };

  return (
    <AnimatePresence>
      {show && (
        <motion.div
          initial={{ opacity: 0, y: -10 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -10 }}
          transition={{ duration: 0.3, ease: [0.25, 0.1, 0.25, 1] }}
          className="inline-flex items-center gap-2 px-3 py-1.5 rounded-full bg-background-hover border border-border-strong text-[12px] text-text-secondary hover:text-text-primary hover:bg-white/5 transition-colors cursor-pointer group mb-8 mx-auto"
        >
          <Info className="w-3.5 h-3.5 text-accent-blue" />
          <span>Local execution — no cloud</span>
          <button 
            onClick={(e) => { e.stopPropagation(); handleDismiss(); }}
            className="ml-1 opacity-0 group-hover:opacity-100 p-0.5 hover:bg-white/5 rounded-full transition-all"
          >
            <X className="w-3 h-3" />
          </button>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
