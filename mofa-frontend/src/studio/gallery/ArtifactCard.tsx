import React from 'react';
import { FileText, Music, Video, Code, FileJson, Sparkles } from 'lucide-react';

export interface ArtifactItem {
  id: string;
  name: string;
  type: 'audio' | 'video' | 'report' | 'data';
  path: string;
  size: string;
  createdAt: string;
  locality: 'local' | 'cloud';
  scenario: string;
  previewUrl?: string;
  content?: string;
}

interface ArtifactCardProps {
  artifact: ArtifactItem;
  onSelect: (artifact: ArtifactItem) => void;
}

export function ArtifactCard({ artifact, onSelect }: ArtifactCardProps) {
  const getIcon = () => {
    switch (artifact.type) {
      case 'audio':
        return <Music className="w-5 h-5 text-accent-cyan" />;
      case 'video':
        return <Video className="w-5 h-5 text-purple-400" />;
      case 'report':
        return <FileText className="w-5 h-5 text-accent-green" />;
      case 'data':
        return <FileJson className="w-5 h-5 text-amber-400" />;
      default:
        return <Code className="w-5 h-5 text-text-secondary" />;
    }
  };

  return (
    <div
      onClick={() => onSelect(artifact)}
      className="group relative flex flex-col p-4 bg-background-secondary/60 hover:bg-background-hover border border-border-subtle hover:border-border-strong rounded-xl cursor-pointer transition-all duration-200 shadow-sm hover:shadow-md"
    >
      <div className="flex items-start justify-between mb-3">
        <div className="p-2.5 rounded-lg bg-white/5 group-hover:bg-white/10 transition-colors">
          {getIcon()}
        </div>
        <span
          className={`px-2 py-0.5 text-[10px] font-mono rounded-full border ${
            artifact.locality === 'local'
              ? 'bg-accent-green/10 border-accent-green/30 text-accent-green'
              : 'bg-amber-400/10 border-amber-400/30 text-amber-400'
          }`}
        >
          {artifact.locality === 'local' ? 'LOCAL $0.00' : 'CLOUD'}
        </span>
      </div>

      <h4 className="text-sm font-medium text-text-primary group-hover:text-accent-cyan transition-colors truncate mb-1">
        {artifact.name}
      </h4>

      <p className="text-[11px] text-text-dim truncate mb-3">{artifact.scenario}</p>

      <div className="mt-auto flex items-center justify-between text-[10px] font-mono text-text-dim border-t border-border-subtle/50 pt-2.5">
        <span>{artifact.size}</span>
        <span>{artifact.createdAt}</span>
      </div>
    </div>
  );
}
