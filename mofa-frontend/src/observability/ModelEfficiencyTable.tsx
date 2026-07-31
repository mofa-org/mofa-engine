import React from 'react';
import { Card } from '../shared/Card';
import { ModelCard } from '../engine/types';

interface ModelEfficiencyTableProps {
  capabilities?: ModelCard[];
}

export function ModelEfficiencyTable({ capabilities = [] }: ModelEfficiencyTableProps) {
  const modelsToDisplay = capabilities.length > 0
    ? capabilities
    : [
        {
          id: 'gemma3:4b',
          name: 'Gemma 3 4B',
          provider: 'Ollama',
          cost_tier: 'free',
          status: 'hot',
        },
        {
          id: 'kokoro',
          name: 'Kokoro TTS v1.0',
          provider: 'Kokoro Local',
          cost_tier: 'free',
          status: 'hot',
        },
        {
          id: 'accounts/fireworks/models/deepseek-v4-flash',
          name: 'DeepSeek v4 Flash',
          provider: 'Fireworks AI',
          cost_tier: 'low',
          status: 'hot',
        },
      ];

  return (
    <Card className="p-6">
      <div className="flex items-center justify-between mb-4">
        <h4 className="text-[12px] font-semibold uppercase tracking-wider text-text-dim">
          Model Efficiency & Capabilities Registry
        </h4>
        <span className="text-[11px] text-text-dim font-mono">
          {capabilities.length > 0 ? `${capabilities.length} active models` : 'default registry'}
        </span>
      </div>

      <div className="overflow-x-auto">
        <table className="w-full text-left text-[12px]">
          <thead>
            <tr className="border-b border-border-subtle text-text-dim font-medium">
              <th className="pb-3 font-normal">Model</th>
              <th className="pb-3 font-normal">Provider</th>
              <th className="pb-3 font-normal">Capability</th>
              <th className="pb-3 font-normal text-right">Cost Tier</th>
              <th className="pb-3 font-normal text-right">Status</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border-subtle font-mono text-[11.5px]">
            {modelsToDisplay.map((m: any) => {
              const isLocal = (m.provider || '').toLowerCase().includes('ollama') || 
                              (m.provider || '').toLowerCase().includes('kokoro') || 
                              (m.provider || '').toLowerCase().includes('local');
              return (
                <tr key={m.id || m.name} className="hover:bg-background-hover transition-colors">
                  <td className="py-3 font-sans font-medium text-text-primary flex items-center gap-2.5">
                    <span
                      className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                        isLocal ? 'bg-brand' : 'bg-accent-yellow'
                      }`}
                    />
                    <span>{m.name || m.id.split('/').pop()}</span>
                  </td>
                  <td className="py-3 text-text-secondary font-sans">{m.provider || 'Local Engine'}</td>
                  <td className="py-3">
                    <span className="text-[10px] font-mono text-text-dim">
                      {Array.isArray(m.capabilities) ? m.capabilities.join(', ') : m.capability || 'Chat'}
                    </span>
                  </td>
                  <td className="py-3 text-right text-text-secondary">
                    <span className="capitalize">{m.cost_tier || (isLocal ? 'Free' : 'Paid')}</span>
                  </td>
                  <td className="py-3 text-right font-medium text-text-primary">
                    <span className="text-[10px] px-2 py-0.5 rounded bg-accent-green/10 text-accent-green border border-accent-green/20">
                      {m.status || 'Active'}
                    </span>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </Card>
  );
}
