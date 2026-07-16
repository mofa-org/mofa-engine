export type Capability = 'Chat' | 'Tts' | 'Asr' | 'ImageGen' | 'VideoGen' | 'Vlm' | 'Embedding';
export type ModelStatus = 'Cold' | 'Warming' | 'Hot' | 'Busy' | 'Failed';
export type ModelResidency = 'Unknown' | 'Unloaded' | 'Loading' | 'Loaded' | 'Unloading' | 'Remote';
export type ModelAvailability = 'Discovered' | 'Configured' | 'Unavailable';
export type ExecutionState = 'Idle' | 'Active' | 'Overloaded';
export type CostTier = 'Free' | 'Low' | 'Medium' | 'High';
export type CircuitState = 'Closed' | 'Open' | 'HalfOpen';
export type BackendHealth = 'Healthy' | 'Degraded' | 'Unhealthy';
export type FallbackPolicy = 'capability_only' | 'disabled' | 'allow_named';

export interface HealthResponse {
  status: string;
  version: string;
  uptime_secs: number;
}

export interface ProviderHealth {
  name: string;
  healthy: boolean;
  circuit_state: CircuitState;
}

export interface Backend {
  name: string;
  kind: string;
  health: BackendHealth;
  circuit_state: CircuitState;
  features: string[];
}

export interface EngineStatus {
  total_models: number;
  loaded_models: number;
  providers: number;
  memory_used_bytes: number;
  memory_budget_bytes: number;
  uptime_secs: number;
  provider_health: ProviderHealth[];
  backends: Backend[];
}

export interface ModelCard {
  id: string;
  name: string;
  provider: string;
  capability: Capability;
  capabilities: Capability[];
  status: ModelStatus;
  availability: ModelAvailability;
  residency: ModelResidency;
  execution: ExecutionState;
  cost_tier: CostTier;
  context_window?: number;
  memory_estimate_bytes?: number;
}

export interface InferenceRequest {
  capability: Capability;
  model: string | null;
  messages: Array<{ role: string; content: string }>;
  hint_next?: string;
  params: Record<string, unknown>;
  app_id: string;
  session_id: string | null;
  fallback_policy: FallbackPolicy;
}

export interface InferenceResponse {
  text: string;
  file: string | null;
  model_used: string;
  provider: string;
  duration_ms: number;
  request_id: string;
  tokens_used: number;
  fallback_used: boolean;
  routing_reason: string;
  candidates_considered?: number;
}

export interface EngineError {
  error: string;
  detail: string;
}

export type EngineResult<T> = 
  | { success: true; data: T }
  | { success: false; type: 'network' | 'http'; error: string; detail?: string };

export type EngineEventType = 'RequestStarted' | 'RequestCompleted' | 'ModelStatusChanged' | 'ModelResidencyChanged' | 'MemoryChanged' | 'ProviderHealthChanged' | 'DiscoveryCompleted';

export interface EngineEvent {
  type: EngineEventType;
  data: any;
  timestamp: number;
}
