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

  async invoke(payload: InferenceRequest) {
    return this.request<InferenceResponse>('/v1/invoke', {
      method: 'POST',
      body: JSON.stringify(payload)
    });
  }

  async refreshDiscovery() {
    return this.request<EngineStatus>('/v1/discovery/refresh', {
      method: 'POST'
    });
  }

  subscribeEvents(handler: (e: EngineEvent) => void): () => void {
    const eventSource = new EventSource(`${this.baseUrl}/v1/events`);
    
    eventSource.onmessage = (event) => {
      try {
        const raw = JSON.parse(event.data);
        // Real engine uses serde tag="type" rename_all="snake_case"
        // so type comes as "request_started", "model_status_changed", etc.
        const snakeType: string = raw.type || '';
        // Convert snake_case to PascalCase: "request_started" → "RequestStarted"
        const pascalType = snakeType.replace(/(^|_)([a-z])/g, (_: string, __: string, c: string) => c.toUpperCase());
        // Everything except 'type' goes into data
        const { type: _, ...data } = raw;
        handler({ type: pascalType as any, data, timestamp: Date.now() });
      } catch (e) {
        console.error("Failed to parse SSE", e);
      }
    };
    
    return () => {
      eventSource.close();
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
