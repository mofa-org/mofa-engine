import {
  HealthResponse,
  EngineStatus,
  ModelCard,
  InferenceRequest,
  InferenceResponse,
  EngineResult,
  EngineEvent
} from './types';
import { IEngineClient } from './index';

export class RealEngineClient implements IEngineClient {
  private baseUrl: string;

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl.replace(/\/$/, '');
  }

  setBaseUrl(url: string) {
    this.baseUrl = url.replace(/\/$/, '');
  }

  getBaseUrl() {
    return this.baseUrl;
  }

  private async request<T>(path: string, options?: RequestInit): Promise<EngineResult<T>> {
    try {
      const response = await fetch(`${this.baseUrl}${path}`, {
        ...options,
        headers: {
          'Content-Type': 'application/json',
          ...(options?.headers || {})
        }
      });

      if (!response.ok) {
        let detail = response.statusText;
        let error = `HTTP ${response.status}`;
        try {
          const body = await response.json();
          if (body.error) error = body.error;
          if (body.detail) detail = body.detail;
        } catch {
          // ignore parsing error if it's not JSON
        }
        return { success: false, type: 'http', error, detail };
      }

      const data = await response.json();
      return { success: true, data };
    } catch (e) {
      return { success: false, type: 'network', error: 'Engine unreachable', detail: (e as Error).message };
    }
  }

  async getHealth() {
    return this.request<HealthResponse>('/health');
  }

  async getStatus() {
    return this.request<EngineStatus>('/v1/status');
  }

  async getCapabilities() {
    return this.request<ModelCard[]>('/v1/capabilities');
  }

  private ensureTraceId(payload: InferenceRequest) {
    if (!payload.trace_id) {
      payload.trace_id = Array.from({length: 32}, () => Math.floor(Math.random()*16).toString(16)).join('');
    }
  }

  async invoke(payload: InferenceRequest) {
    this.ensureTraceId(payload);
    return this.request<InferenceResponse>('/v1/invoke', {
      method: 'POST',
      body: JSON.stringify(payload)
    });
  }

  async streamInvoke(payload: InferenceRequest, onChunk: (text: string) => void): Promise<EngineResult<InferenceResponse>> {
    this.ensureTraceId(payload);
    try {
      const response = await fetch(`${this.baseUrl}/v1/invoke/stream`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
      });
      
      if (!response.ok) {
        let detail = response.statusText;
        let error = `HTTP ${response.status}`;
        try {
          const body = await response.json();
          if (body.error) error = body.error;
          if (body.detail) detail = body.detail;
        } catch {
          // ignore non-json response body
        }
        return { success: false, type: 'http', error, detail };
      }

      if (!response.body) {
        return { success: false, type: 'network', error: 'No response body' };
      }

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      let finalResponse: InferenceResponse | null = null;

      const startTime = Date.now();
      let accumulatedText = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';

        for (const line of lines) {
          if (line.startsWith('data:')) {
            const dataStr = line.slice(5).trim();
            if (dataStr === '[DONE]') continue;
            try {
              const chunk = JSON.parse(dataStr);
              const textChunk = typeof chunk === 'string' ? chunk : (chunk.text || chunk.delta || chunk.content || chunk.chunk || '');
              if (textChunk) {
                accumulatedText += textChunk;
                onChunk(textChunk);
              }
              if (chunk.final_response) {
                finalResponse = chunk.final_response;
              }
            } catch {
              // Plain string SSE chunk fallback
              if (dataStr) {
                accumulatedText += dataStr;
                onChunk(dataStr);
              }
            }
          }
        }
      }

      if (!finalResponse && accumulatedText) {
        finalResponse = {
          text: accumulatedText,
          file: null,
          model_used: payload.model || 'local',
          provider: payload.locality === 'cloud' ? 'fireworks' : 'ollama',
          duration_ms: Date.now() - startTime,
          request_id: payload.trace_id || 'req-' + Date.now(),
          tokens_used: accumulatedText.split(/\s+/).length * 2,
          fallback_used: false,
          routing_reason: 'capability_match'
        };
      } else if (finalResponse && !finalResponse.text && accumulatedText) {
        finalResponse.text = accumulatedText;
      }

      if (finalResponse) {
        return { success: true, data: finalResponse };
      } else {
        return { success: false, type: 'network', error: 'Stream ended without output' };
      }
    } catch (e) {
      return { success: false, type: 'network', error: 'Engine unreachable', detail: (e as Error).message };
    }
  }

  async refreshDiscovery() {
    return this.request<EngineStatus>('/v1/discovery/refresh', {
      method: 'POST'
    });
  }

  async getMetrics(): Promise<EngineResult<string>> {
    try {
      const response = await fetch(`${this.baseUrl}/metrics`);
      if (!response.ok) return { success: false, type: 'http', error: `HTTP ${response.status}` };
      const text = await response.text();
      return { success: true, data: text };
    } catch (e) {
      return { success: false, type: 'network', error: (e as Error).message };
    }
  }

  // Shared singleton EventSource — browsers limit concurrent connections per host
  // to ~6 (HTTP/1.1). Opening one EventSource per subscriber was exhausting the
  // pool, causing subsequent fetch() calls (like TTS invoke) to queue forever.
  private sharedEventSource: EventSource | null = null;
  private eventHandlers = new Set<(e: EngineEvent) => void>();

  subscribeEvents(handler: (e: EngineEvent) => void): () => void {
    this.eventHandlers.add(handler);

    // Lazily open the shared SSE connection on first subscriber
    if (!this.sharedEventSource) {
      const url = `${this.baseUrl}/v1/events`;
      if (import.meta.env.DEV) console.log(`[SSE] Opening shared EventSource → ${url}`);
      const es = new EventSource(url);
      es.onopen = () => {
        if (import.meta.env.DEV) console.log(`[SSE] Connection opened, readyState=${es.readyState}, handlers=${this.eventHandlers.size}`);
      };
      es.onmessage = (event) => {
        try {
          const raw = JSON.parse(event.data);
          const snakeType: string = raw.type || '';
          const pascalType = snakeType.replace(/(^|_)([a-z])/g, (_: string, __: string, c: string) => c.toUpperCase());
          const { type: _, ...data } = raw;
          const parsed: EngineEvent = { type: pascalType as any, data, timestamp: Date.now() };
          if (import.meta.env.DEV) console.log(`[SSE] Event received: ${pascalType}, handlers=${this.eventHandlers.size}`);
          // Fan out to all registered handlers
          this.eventHandlers.forEach(h => h(parsed));
        } catch (e) {
          // Keep-alive pings or malformed data — ignore silently
          if (event.data !== 'ping') {
            console.error("[SSE] Failed to parse:", event.data, e);
          }
        }
      };
      es.onerror = (err) => {
        console.warn(`[SSE] Connection error, readyState=${es.readyState}`, err);
      };
      this.sharedEventSource = es;
    }

    // Return unsubscribe function
    return () => {
      this.eventHandlers.delete(handler);
      // Close the shared EventSource when no subscribers remain
      if (this.eventHandlers.size === 0 && this.sharedEventSource) {
        this.sharedEventSource.close();
        this.sharedEventSource = null;
      }
    };
  }
  
  getAudioUrl(filename: string): string {
    return `${this.baseUrl}/v1/files/${filename}`;
  }

  async fetchAudio(filename: string): Promise<Blob> {
    const res = await fetch(this.getAudioUrl(filename));
    if (!res.ok) throw new Error(`Failed to fetch audio: ${res.statusText}`);
    return res.blob();
  }
}
