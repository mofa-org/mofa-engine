import { useState } from 'react';
import { PipelinePhase } from '../studio/usePipeline';

const HISTORY_KEY = 'mofa_history';
const DRAFT_KEY = 'mofa_compose_draft';

export function useHistory() {
  const [history, setHistory] = useState<PipelinePhase[]>(() => {
    const raw = localStorage.getItem(HISTORY_KEY);
    if (raw) {
      try {
        return JSON.parse(raw);
      } catch (e) {
        console.error('Failed to parse history', e);
      }
    }
    return [];
  });

  const saveToHistory = (phase: PipelinePhase) => {
    if (phase.status !== 'done') return;
    setHistory(prev => {
      const next = [phase, ...prev].slice(0, 50); // Keep last 50
      localStorage.setItem(HISTORY_KEY, JSON.stringify(next));
      return next;
    });
  };

  const clearHistory = () => {
    setHistory([]);
    localStorage.removeItem(HISTORY_KEY);
  };

  return { history, saveToHistory, clearHistory };
}

export function useDraft() {
  const [draft, setDraft] = useState<string>(() => {
    return localStorage.getItem(DRAFT_KEY) || '';
  });

  const saveDraft = (text: string) => {
    setDraft(text);
    localStorage.setItem(DRAFT_KEY, text);
  };

  return { draft, saveDraft };
}
