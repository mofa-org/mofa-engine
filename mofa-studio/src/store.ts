// Studio store: the asset library (every generated image/video), persisted to
// localStorage so it survives restarts. Drives the Assets page and Workflow canvas.
//
// TODO(persistence): move to SQLite via `tauri-plugin-sql` for full-text search and
// large libraries; localStorage is the first cut.

import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { GenResult } from "./services/engine";

export type AssetKind = "image" | "video";

export type Asset = {
  id: string;
  kind: AssetKind;
  prompt: string;
  /** Absolute engine artifact path (converted to a src via `assetSrc`). */
  path: string;
  provider?: string | null;
  local?: boolean | null;
  costUsd?: number | null;
  durationMs?: number | null;
  createdAt: number;
};

type StudioState = {
  assets: Asset[];
  addAsset: (kind: AssetKind, prompt: string, result: GenResult) => Asset | null;
  removeAsset: (id: string) => void;
  clearAssets: () => void;
};

export const useStudio = create<StudioState>()(
  persist(
    (set) => ({
      assets: [],
      addAsset: (kind, prompt, result) => {
        if (!result.ok || !result.path) return null;
        const asset: Asset = {
          id: crypto.randomUUID(),
          kind,
          prompt,
          path: result.path,
          provider: result.provider,
          local: result.local,
          costUsd: result.cost_usd,
          durationMs: result.duration_ms,
          createdAt: Date.now(),
        };
        set((s) => ({ assets: [asset, ...s.assets] }));
        return asset;
      },
      removeAsset: (id) => set((s) => ({ assets: s.assets.filter((a) => a.id !== id) })),
      clearAssets: () => set({ assets: [] }),
    }),
    { name: "mofa-studio-assets" },
  ),
);
