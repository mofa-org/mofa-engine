import React from 'react';
import { X, Download, Play, FileText, Music, Video, FileJson } from 'lucide-react';
import { ArtifactItem } from './ArtifactCard';

interface ArtifactPreviewProps {
  artifact: ArtifactItem | null;
  onClose: () => void;
}

export function ArtifactPreview({ artifact, onClose }: ArtifactPreviewProps) {
  if (!artifact) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/60 backdrop-blur-sm">
      <div className="relative w-full max-w-2xl bg-background-secondary border border-border-strong rounded-2xl shadow-2xl overflow-hidden flex flex-col max-h-[85vh]">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-border-subtle bg-background-primary/50">
          <div className="flex items-center gap-3">
            <span className="text-sm font-semibold text-text-primary">{artifact.name}</span>
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
          <button
            onClick={onClose}
            className="p-1.5 text-text-dim hover:text-text-primary hover:bg-white/5 rounded-lg transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Content Body */}
        <div className="flex-1 overflow-y-auto p-6">
          {artifact.type === 'audio' && (
            <div className="flex flex-col items-center justify-center py-8">
              <div className="w-16 h-16 rounded-full bg-accent-cyan/10 flex items-center justify-center mb-4">
                <Music className="w-8 h-8 text-accent-cyan" />
              </div>
              <audio
                controls
                src={artifact.previewUrl || `/v1/artifacts/${artifact.name}`}
                className="w-full max-w-md mt-4"
              >
                Your browser does not support the audio element.
              </audio>
              <p className="text-xs text-text-dim mt-4 font-mono">Location: {artifact.path}</p>
            </div>
          )}

          {artifact.type === 'video' && (
            <div className="flex flex-col items-center justify-center">
              <video
                controls
                src={artifact.previewUrl || `/v1/artifacts/${artifact.name}`}
                className="w-full max-h-[50vh] rounded-xl bg-black"
              >
                Your browser does not support the video element.
              </video>
              <p className="text-xs text-text-dim mt-4 font-mono">Location: {artifact.path}</p>
            </div>
          )}

          {(artifact.type === 'report' || artifact.type === 'data') && (
            <div className="bg-background-primary/80 border border-border-subtle rounded-xl p-4 font-mono text-xs text-text-secondary whitespace-pre-wrap overflow-x-auto">
              {artifact.content || `# ${artifact.name}\n\nArtifact file created at: ${artifact.path}\nSize: ${artifact.size}`}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between px-6 py-3 border-t border-border-subtle bg-background-primary/30 text-xs text-text-dim">
          <span>{artifact.scenario} · {artifact.size}</span>
          <a
            href={artifact.previewUrl || `/v1/artifacts/${artifact.name}`}
            download={artifact.name}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-accent-cyan/10 hover:bg-accent-cyan/20 border border-accent-cyan/30 text-accent-cyan rounded-lg transition-colors font-medium text-xs"
          >
            <Download className="w-3.5 h-3.5" />
            <span>Download</span>
          </a>
        </div>
      </div>
    </div>
  );
}
