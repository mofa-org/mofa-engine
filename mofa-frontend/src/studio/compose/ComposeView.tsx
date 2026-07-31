import React, { useState } from 'react';
import { motion } from 'framer-motion';
import { motionVariants } from '../../lib/motion';
import { Card } from '../../shared/Card';
import { Button } from '../../shared/Button';
import { ChevronDown, Play, Sparkles, Languages, Cpu } from 'lucide-react';
import { useEngineConnection } from '../../engine/useEngineConnection';
import { useDraft } from '../../storage/useHistory';
import { OnboardingBanner } from './OnboardingBanner';

interface ComposeViewProps {
  onStart: (article: string, options: { systemPrompt: string; voice: string; locality?: 'local' | 'cloud' | 'auto'; model?: string | null }) => void;
}

const SAMPLES = {
  'AI News': "A new study reveals that artificial intelligence models are increasingly capable of writing highly optimized code, but struggle with contextual nuances in legacy systems. Researchers at MIT found a 40% increase in developer productivity when pairing AI with human review.",
  'Science': "The James Webb Space Telescope has captured a stunning new image of the Pillars of Creation. The near-infrared camera peered through dust clouds to reveal nascent stars forming in the gas pillars, located 6,500 light-years away in the Eagle Nebula.",
  'Short Story': "The rain hasn't stopped for three days. Inside the small café on 5th Street, Elias watched the water run down the windowpane, distorting the neon signs outside. He clutched his coffee mug, waiting for a message that might never come."
};

export function ComposeView({ onStart }: ComposeViewProps) {
  const { draft, saveDraft } = useDraft();
  const [article, setArticle] = useState(draft || SAMPLES['AI News']);
  const [voice, setVoice] = useState('Xiaoxiao');
  const [providerOption, setProviderOption] = useState<'local' | 'cloud'>('local');
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [systemPrompt, setSystemPrompt] = useState('Rewrite as a Chinese podcast script, under 200 characters. Output only Chinese.');
  
  const { state: engineState } = useEngineConnection();
  
  const handleArticleChange = (val: string) => {
    setArticle(val);
    saveDraft(val);
  };
  
  const handleGenerate = () => {
    if (!article.trim() || engineState !== 'connected') return;
    const locality = providerOption === 'cloud' ? 'cloud' : 'local';
    const model = providerOption === 'cloud' ? 'fireworks/accounts/fireworks/models/deepseek-v4-flash' : null;
    onStart(article, { systemPrompt, voice, locality, model });
  };
  
  const isButtonDisabled = !article.trim() || engineState !== 'connected';
  
  return (
    <motion.div 
      className="flex-1 w-full h-full overflow-y-auto"
      variants={motionVariants.enter}
      initial="initial"
      animate="animate"
      exit="exit"
    >
      <div className="w-full max-w-[720px] mx-auto px-6 py-12 flex flex-col min-h-full">
        <div className="flex flex-col items-center mb-8">
        <h1 className="text-[24px] font-semibold text-text-primary tracking-tight mb-2">MoFA Engine Demonstration</h1>
        <p className="text-[13px] text-text-secondary max-w-lg text-center leading-relaxed">Powered by the MoFA Engine — intelligent local model orchestration, preflight routing, and dynamic memory management.</p>
      </div>

      <OnboardingBanner />

      <Card className="p-6 relative group mb-4 transition-all duration-300 focus-within:border-accent-cyan/50 focus-within:shadow-[0_0_30px_rgba(6,182,212,0.1)]">
        <div className="absolute inset-0 bg-accent-cyan/5 blur-3xl opacity-0 group-focus-within:opacity-100 transition-opacity pointer-events-none" />
        
        <div className="relative z-10 flex flex-col">
          <textarea
            value={article}
            onChange={e => handleArticleChange(e.target.value)}
            onKeyDown={e => {
              if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
                handleGenerate();
              }
            }}
            className="w-full min-h-[200px] max-h-[50vh] bg-transparent resize-y text-[15px] text-text-primary placeholder:text-text-dim focus:outline-none leading-relaxed"
            placeholder="Paste an English article here — a news piece, a blog post, an essay… or try a sample below ↓"
            aria-label="Article text"
          />
          <div className="flex justify-between items-center mt-4 pt-4 border-t border-border-subtle">
            <div className="flex items-center gap-3">
              <span className="text-[12px] text-text-dim hidden sm:inline-block">Try a sample:</span>
              <div className="flex gap-2">
                {(Object.keys(SAMPLES) as Array<keyof typeof SAMPLES>).map(key => (
                  <button
                    key={key}
                    onClick={() => handleArticleChange(SAMPLES[key])}
                    className="px-3 py-1 rounded-full bg-background-hover hover:bg-white/5 text-[13px] text-text-secondary hover:text-text-primary transition-colors focus:outline-none focus:ring-2 focus:ring-accent-cyan"
                    aria-label={`Load sample ${key}`}
                  >
                    {key}
                  </button>
                ))}
              </div>
            </div>
            <span className="text-[13px] font-mono text-text-dim shrink-0">{article.length} chars</span>
          </div>
        </div>
      </Card>
      
      {article.length > 4000 && (
        <p className="text-[13px] text-accent-yellow/80 mb-4 px-2">Long articles take a little longer.</p>
      )}

      <div className="flex items-center gap-4 mb-8 flex-wrap">
        {/* Provider Locality Selector */}
        <div className="relative">
          <select 
            value={providerOption}
            onChange={e => setProviderOption(e.target.value as 'local' | 'cloud')}
            className="appearance-none bg-background-hover border border-border-strong rounded-[var(--radius-small)] pl-8 pr-8 py-2 text-[13px] text-text-primary focus:outline-none focus:border-accent-blue font-medium"
          >
            <option value="local">Local Hardware (Ollama)</option>
            <option value="cloud">Cloud Financial (Fireworks AI)</option>
          </select>
          <Cpu className="w-4 h-4 text-accent-blue absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
          <ChevronDown className="w-4 h-4 text-text-secondary absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
        </div>

        {/* Voice Selector */}
        <div className="relative">
          <select 
            value={voice}
            onChange={e => setVoice(e.target.value)}
            className="appearance-none bg-background-hover border border-border-strong rounded-[var(--radius-small)] pl-8 pr-8 py-2 text-[13px] text-text-primary focus:outline-none focus:border-accent-purple"
          >
            {['Xiaoxiao', 'Yunxi', 'Xiaoni', 'Nova', 'Alloy', 'Echo'].map(v => (
              <option key={v} value={v} className="bg-background-secondary">{v}</option>
            ))}
          </select>
          <Sparkles className="w-4 h-4 text-accent-purple absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
          <ChevronDown className="w-4 h-4 text-text-secondary absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
        </div>
        
        {/* Language Selector */}
        <div className="relative">
          <select disabled className="appearance-none bg-background-hover border border-border-strong rounded-[var(--radius-small)] pl-8 pr-8 py-2 text-[13px] text-text-secondary opacity-70 cursor-not-allowed">
            <option>Chinese</option>
            <option disabled>Spanish (soon)</option>
          </select>
          <Languages className="w-4 h-4 text-text-secondary absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
          <ChevronDown className="w-4 h-4 text-text-secondary absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
        </div>
        
        <button 
          onClick={() => setShowAdvanced(!showAdvanced)}
          className="ml-auto flex items-center gap-1 text-[13px] text-text-dim hover:text-text-secondary transition-colors"
        >
          Advanced <ChevronDown className={`w-4 h-4 transition-transform ${showAdvanced ? 'rotate-180' : ''}`} />
        </button>
      </div>

      {showAdvanced && (
        <motion.div 
          initial={{ opacity: 0, height: 0 }}
          animate={{ opacity: 1, height: 'auto' }}
          transition={{ duration: 0.3, ease: [0.25, 0.1, 0.25, 1] }}
          className="mb-8 overflow-hidden"
        >
          <label className="block text-[13px] font-medium text-text-secondary mb-2">System Prompt</label>
          <textarea
            value={systemPrompt}
            onChange={e => setSystemPrompt(e.target.value)}
            className="w-full h-20 bg-background-secondary border border-border-strong rounded-[var(--radius-small)] p-3 text-[13px] font-mono text-text-primary focus:outline-none focus:border-accent-blue/50"
          />
        </motion.div>
      )}

      <div className="flex flex-col items-center gap-3 mt-auto">
        <Button 
          disabled={isButtonDisabled}
          onClick={handleGenerate}
          className="w-full max-w-[320px] h-12 text-[14px] gap-2 font-semibold"
          title={!article.trim() ? "Paste an article first" : engineState !== 'connected' ? "Engine is offline" : ""}
        >
          <Play className="w-4 h-4" />
          Generate Podcast
        </Button>
        <span className="text-[13px] text-text-dim">
          {providerOption === 'cloud' ? '☁️ Fireworks AI DeepSeek Cloud · Live USD Billing' : '~18 seconds · fully local · Cmd ↵'}
        </span>
      </div>
      </div>
    </motion.div>
  );
}
