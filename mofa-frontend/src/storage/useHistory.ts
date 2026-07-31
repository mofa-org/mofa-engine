import { useState, useCallback } from 'react';
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

  const saveToHistory = useCallback((phase: PipelinePhase) => {
    if (phase.status !== 'done') return;
    setHistory(prev => {
      // Avoid duplicate saves for the exact same phase
      const phaseId = phase.tts?.requestId || phase.chat?.requestId;
      if (prev.length > 0) {
        const firstId = prev[0].status === 'done' ? (prev[0].tts?.requestId || prev[0].chat?.requestId) : null;
        if (firstId && firstId === phaseId) return prev;
      }
      const next = [phase, ...prev].slice(0, 50); // Keep last 50
      localStorage.setItem(HISTORY_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const clearHistory = useCallback(() => {
    setHistory([]);
    localStorage.removeItem(HISTORY_KEY);
  }, []);

  return { history, saveToHistory, clearHistory };
}

export function useDraft() {
  const [draft, setDraft] = useState<string>(() => {
    return localStorage.getItem(DRAFT_KEY) || '';
  });

  const saveDraft = useCallback((val: string) => {
    setDraft(val);
    localStorage.setItem(DRAFT_KEY, val);
  }, []);

  return { draft, saveDraft };
}
