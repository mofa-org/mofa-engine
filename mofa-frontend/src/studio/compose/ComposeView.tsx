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
  onStart: (article: string, options: { systemPrompt: string; voice: string; locality?: 'local' | 'cloud' | 'auto'; model?: string | null; scenarioId?: string; scenarioName?: string; apiKey?: string }) => void;
}

interface ScenarioPreset {
  id: string;
  name: string;
  badge: string;
  pipeline: string;
  defaultText: string;
  systemPrompt: string;
  voice: string;
  locality: 'local' | 'cloud' | 'auto';
}

const SCENARIO_PRESETS: ScenarioPreset[] = [
  {
    id: 's6-podcast',
    name: 'S6 Podcast Matrix',
    badge: 'Flagship Audio',
    pipeline: 'English Article → Chinese Podcast Rewrite (hint_next=tts) → Xiaoxiao TTS',
    defaultText: "A new study reveals that artificial intelligence models are increasingly capable of writing highly optimized code, but struggle with contextual nuances in legacy systems. Researchers at MIT found a 40% increase in developer productivity when pairing AI with human review.",
    systemPrompt: "Translate and rewrite this English article into an engaging, natural conversational Chinese podcast dialogue between two hosts, under 200 characters. Output ONLY Chinese spoken dialogue (中文).",
    voice: 'Xiaoxiao',
    locality: 'local'
  },
  {
    id: 's4-explainer',
    name: 'S4 Explainer Video',
    badge: 'Flagship Video',
    pipeline: 'Topic → Script → ImageGen Visuals → TTS Narration → MP4',
    defaultText: "Quantum computing leverages qubits in superposition to evaluate vast computational search spaces in parallel rather than sequentially.",
    systemPrompt: "Write a 3-sentence spoken video narration script. Output ONLY spoken words.",
    voice: 'Alloy',
    locality: 'local'
  },
  {
    id: 's2-review',
    name: 'S2 Code Review',
    badge: 'Deep Reasoning',
    pipeline: 'Git Diff → Responses API (effort=high) → Thought Chain → Report',
    defaultText: "diff --git a/auth/jwt.py b/auth/jwt.py\n@@ -12,2 +12,2 @@\n-    claims['exp'] = datetime.utcnow() + timedelta(hours=1)\n+    # TODO: Temporarily disable expiry for testing\n+    pass",
    systemPrompt: "Perform a rigorous security and performance code review. Stream deep reasoning tokens.",
    voice: 'Echo',
    locality: 'local'
  },
  {
    id: 's1-meeting',
    name: 'S1 Meeting Minutes',
    badge: 'Enterprise Audio',
    pipeline: 'Meeting Audio → ASR Transcribe → LLM Minutes + Action Items → 30s Audio Brief',
    defaultText: "Speaker 1 (Alice): 'We must lock enterprise data to local models by Friday.'\nSpeaker 2 (Bob): 'Kokoro TTS achieves 85ms latency on Apple Silicon.'\nSpeaker 3 (Carol): 'Zero data egress verified under prefer=local.'",
    systemPrompt: "Extract executive meeting minutes with Decisions, Action Items, and Risks. Conclude with '## Executive Audio Brief' containing 2-3 concise spoken sentences (under 60 words, clean plain text without asterisks or markdown symbols) for a 30-second executive audio brief.",
    voice: 'Nova',
    locality: 'local'
  },
  {
    id: 's5-privacy',
    name: 'S5 Privacy Moat',
    badge: 'Air-Gapped Local',
    pipeline: 'Confidential Query → 100% Local Inference (prefer=local) → Zero Data Egress',
    defaultText: "Analyze this proprietary internal financial ledger: Q3 Operating Margin: 34.2%, Total R&D Outlay: $4.2M, Projected Cash Runway: 18 months. Verify key operational metrics.",
    systemPrompt: "You are an air-gapped corporate intelligence assistant. Analyze the confidential data and provide an executive breakdown. Never exfiltrate data.",
    voice: 'Echo',
    locality: 'local'
  },
  {
    id: 's3-doc-ai',
    name: 'S3 Document AI',
    badge: 'Vision VLM',
    pipeline: 'Document / Receipt Image → VLM Multimodal Extraction → Structured JSON',
    defaultText: "Extract total amount, merchant name, date, line items, and tax in valid JSON format from this document.",
    systemPrompt: "You are an expert Document AI assistant. Analyze the document and return strictly valid structured JSON without commentary.",
    voice: 'Alloy',
    locality: 'local'
  },
  {
    id: 's7-race',
    name: 'S7 Provider Race',
    badge: 'Dual-Track Benchmark',
    pipeline: 'Benchmark Prompt → Concurrent Multi-Provider Race → Latency & Cost Matrix',
    defaultText: "Explain quantum entanglement in exactly 2 concise sentences.",
    systemPrompt: "Provide a clear, accurate, 2-sentence response for multi-provider benchmarking.",
    voice: 'Alloy',
    locality: 'auto'
  }
];

export function ComposeView({ onStart }: ComposeViewProps) {
  const { draft, saveDraft } = useDraft();
  const [selectedScenario, setSelectedScenario] = useState<string>('s6-podcast');
  const [article, setArticle] = useState(draft || SCENARIO_PRESETS[0].defaultText);
  const [voice, setVoice] = useState('Xiaoxiao');
  const [providerOption, setProviderOption] = useState<'local' | 'cloud'>('local');
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [systemPrompt, setSystemPrompt] = useState(SCENARIO_PRESETS[0].systemPrompt);
  const [apiKey, setApiKey] = useState('');
  
  const { state: engineState } = useEngineConnection();

  const handleSelectScenario = (preset: ScenarioPreset) => {
    setSelectedScenario(preset.id);
    setArticle(preset.defaultText);
    saveDraft(preset.defaultText);
    setSystemPrompt(preset.systemPrompt);
    setVoice(preset.voice);
    setProviderOption(preset.locality === 'cloud' ? 'cloud' : 'local');
  };
  
  const handleArticleChange = (val: string) => {
    setArticle(val);
    saveDraft(val);
  };
  
  const handleGenerate = () => {
    if (!article.trim() || engineState !== 'connected') return;
    const locality = providerOption === 'cloud' ? 'cloud' : 'local';
    const model = providerOption === 'cloud' ? 'gemini/gemini-3.6-flash' : null;
    const activePreset = SCENARIO_PRESETS.find(p => p.id === selectedScenario);
    onStart(article, { 
      systemPrompt, 
      voice, 
      locality, 
      model,
      scenarioId: selectedScenario,
      scenarioName: activePreset?.name,
      apiKey
    });
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

      {/* Scenario Preset Selector Cards */}
      <div className="mb-4">
        <div className="text-[12px] font-semibold uppercase tracking-wider text-text-dim mb-2 px-1">Select Delivery Scenario:</div>
        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-2 mb-2">
          {SCENARIO_PRESETS.map(preset => {
            const isSelected = selectedScenario === preset.id;
            return (
              <button
                key={preset.id}
                onClick={() => handleSelectScenario(preset)}
                className={`flex flex-col items-start p-2.5 rounded-[var(--radius-small)] border text-left transition-all ${
                  isSelected
                    ? 'border-accent-cyan bg-accent-cyan/10 shadow-[0_0_15px_rgba(6,182,212,0.15)]'
                    : 'border-border-subtle bg-background-card hover:bg-background-hover hover:border-border-strong'
                }`}
              >
                <span className={`text-[12px] font-medium ${isSelected ? 'text-accent-cyan' : 'text-text-primary'}`}>{preset.name}</span>
                <span className="text-[10px] text-text-dim mt-0.5">{preset.badge}</span>
              </button>
            );
          })}
        </div>
        {/* Active Pipeline Indicator */}
        <div className="px-3 py-1.5 rounded-[var(--radius-small)] bg-background-hover/60 border border-border-subtle text-[11px] text-text-secondary flex items-center gap-2">
          <Sparkles className="w-3.5 h-3.5 text-accent-cyan shrink-0" />
          <span className="font-mono">{SCENARIO_PRESETS.find(p => p.id === selectedScenario)?.pipeline}</span>
        </div>
      </div>

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
            className="w-full min-h-[160px] max-h-[50vh] bg-transparent resize-y text-[14px] font-mono text-text-primary placeholder:text-text-dim focus:outline-none leading-relaxed"
            placeholder="Paste text, prompt, or diff here..."
            aria-label="Scenario input"
          />
          <div className="flex justify-between items-center mt-3 pt-3 border-t border-border-subtle">
            <span className="text-[11px] text-text-dim">Preset input loaded for {SCENARIO_PRESETS.find(p => p.id === selectedScenario)?.name}</span>
            <span className="text-[12px] font-mono text-text-dim shrink-0">{article.length} chars</span>
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
            <option value="cloud">Cloud Burst (Google Gemini)</option>
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
          <select 
            value={voice === 'Alloy' || voice === 'Echo' || voice === 'Nova' ? 'en' : 'zh'}
            onChange={e => {
              const lang = e.target.value;
              if (lang === 'zh') {
                setVoice('Xiaoxiao');
                if (selectedScenario === 's6-podcast') {
                  setSystemPrompt("Translate and rewrite this English article into an engaging, natural conversational Chinese podcast dialogue between two hosts, under 200 characters. Output ONLY Chinese spoken dialogue (中文).");
                }
              } else {
                setVoice('Alloy');
                if (selectedScenario === 's6-podcast') {
                  setSystemPrompt("Rewrite this article into a concise, engaging English conversational podcast dialogue between two hosts. Keep under 250 characters. Output ONLY spoken words.");
                }
              }
            }}
            className="appearance-none bg-background-hover border border-border-strong rounded-[var(--radius-small)] pl-8 pr-8 py-2 text-[13px] text-text-primary focus:outline-none focus:border-accent-blue font-medium"
          >
            <option value="zh">Chinese (中文)</option>
            <option value="en">English</option>
          </select>
          <Languages className="w-4 h-4 text-accent-cyan absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
          <ChevronDown className="w-4 h-4 text-text-secondary absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
        </div>
        
        <button 
          onClick={() => setShowAdvanced(!showAdvanced)}
          className="ml-auto flex items-center gap-1 text-[13px] text-text-dim hover:text-text-secondary transition-colors"
        >
          Advanced <ChevronDown className={`w-4 h-4 transition-transform ${showAdvanced ? 'rotate-180' : ''}`} />
        </button>
      </div>

      {providerOption === 'cloud' && (
        <div className="flex items-center gap-3 mb-6">
          <label className="text-[13px] text-text-secondary whitespace-nowrap">Gemini API Key</label>
          <input
            type="password"
            value={apiKey}
            onChange={e => setApiKey(e.target.value)}
            placeholder="AIza..."
            className="flex-1 bg-background-secondary border border-border-strong rounded-[var(--radius-small)] px-3 py-2 text-[13px] font-mono text-text-primary focus:outline-none focus:border-accent-blue placeholder:text-text-dim"
          />
          <span className="text-[11px] text-text-dim">Stored locally only</span>
        </div>
      )}

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
          {selectedScenario === 's6-podcast' ? 'Generate Podcast' :
           selectedScenario === 's4-explainer' ? 'Generate Video' :
           selectedScenario === 's2-review' || selectedScenario === 's2-code-review' ? 'Generate Review' :
           selectedScenario === 's1-meeting' ? 'Generate Brief' :
           selectedScenario === 's5-privacy' ? 'Analyze Confidentially' :
           selectedScenario === 's3-doc-ai' ? 'Extract Document JSON' :
           selectedScenario === 's7-race' ? 'Run Provider Race' :
           'Generate Content'}
        </Button>
        <span className="text-[13px] text-text-dim">
          {providerOption === 'cloud' ? '[CLOUD] Google Gemini 2.5 Flash · Cloud Burst' : '~18 seconds · fully local · Cmd ↵'}
        </span>
      </div>
      </div>
    </motion.div>
  );
}
