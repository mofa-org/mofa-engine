// Typed bridge between the React UI and the Rust commands. Components call these
// instead of `invoke` directly; `chatStream` turns the streaming command into a
// callback of typed chunks.

import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type CapabilityRow = {
  capability: string;
  provider: string;
  available: boolean;
  local: boolean;
};

export type GenResult = {
  ok: boolean;
  path?: string | null;
  provider?: string | null;
  local?: boolean | null;
  cost_usd?: number | null;
  duration_ms?: number | null;
  error?: string | null;
};

export type ChatMessage = { role: "system" | "user" | "assistant"; content: string };

// Mirrors mofa_kernel::StreamChunk (serde tag = "type", snake_case).
export type StreamChunk =
  | { type: "started"; request_id: string; model_used: string; provider: string }
  | { type: "text"; delta: string }
  | { type: "reasoning"; delta: string }
  | {
      type: "completed";
      duration_ms: number;
      tokens_used?: number | null;
      cost_usd?: number | null;
      file?: string | null;
      fallback_used: boolean;
      routing_reason?: string | null;
    }
  | { type: "error"; code?: string; message: string };

export function getCapabilities(): Promise<CapabilityRow[]> {
  return invoke<CapabilityRow[]>("get_capabilities");
}

export function generateImage(prompt: string, size?: string): Promise<GenResult> {
  return invoke<GenResult>("generate_image", { prompt, size });
}

export function generateVideo(
  prompt: string,
  opts?: { resolution?: string; duration?: number; ratio?: string },
): Promise<GenResult> {
  return invoke<GenResult>("generate_video", { prompt, ...opts });
}

/** Turn an engine artifact path into a webview-loadable URL (asset protocol). */
export function assetSrc(path: string): string {
  return convertFileSrc(path);
}

/**
 * Run a streaming chat turn. Chunks are delivered to `onChunk`; the promise
 * resolves when the stream closes. `streamId` keeps concurrent streams separate.
 */
export async function chatStream(
  messages: ChatMessage[],
  onChunk: (chunk: StreamChunk) => void,
): Promise<void> {
  const streamId = crypto.randomUUID();

  // Subscribe *before* invoking so we never miss an early chunk.
  const unlisten = await listen<{ stream_id: string; chunk: StreamChunk }>(
    "chat://chunk",
    (event) => {
      if (event.payload.stream_id === streamId) onChunk(event.payload.chunk);
    },
  );

  try {
    await invoke("chat_stream", { streamId, messages });
  } finally {
    unlisten();
  }
}
