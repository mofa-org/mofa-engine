import { HealthResponse, EngineStatus, ModelCard, InferenceRequest, InferenceResponse, EngineResult, EngineEvent } from './types';
import { RealEngineClient } from './client';
export interface IEngineClient {
  getBaseUrl(): string;
  setBaseUrl(url: string): void;
  getHealth(): Promise<EngineResult<HealthResponse>>;
  getStatus(): Promise<EngineResult<EngineStatus>>;
  getCapabilities(): Promise<EngineResult<ModelCard[]>>;
  invoke(req: InferenceRequest): Promise<EngineResult<InferenceResponse>>;
  streamInvoke(req: InferenceRequest, onChunk: (text: string) => void): Promise<EngineResult<InferenceResponse>>;
  refreshDiscovery(): Promise<EngineResult<EngineStatus>>;
  getMetrics(): Promise<EngineResult<string>>;
  subscribeEvents(handler: (e: EngineEvent) => void): () => void;
  getAudioUrl(filename: string): string;
  fetchAudio(filename: string): Promise<Blob>;
}

export const defaultEngineUrl = import.meta.env.VITE_ENGINE_URL || 'http://127.0.0.1:8420';

export const getStoredEngineUrl = () => {
  return localStorage.getItem('mofa_engine_url') || defaultEngineUrl;
};

import { useState, useEffect } from 'react';

type Listener = () => void;
const listeners = new Set<Listener>();

export const setStoredEngineUrl = (url: string) => {
  localStorage.setItem('mofa_engine_url', url);
  engine.setBaseUrl(url);
  listeners.forEach(l => l());
};

export const useEngineUrl = () => {
  const [url, setUrl] = useState(getStoredEngineUrl());
  useEffect(() => {
    const listener = () => setUrl(getStoredEngineUrl());
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }, []);
  return url;
};

export const engine: IEngineClient = new RealEngineClient(getStoredEngineUrl());
