import { useEffect, useRef, useState } from 'react';
import WaveSurfer from 'wavesurfer.js';
import Hover from 'wavesurfer.js/dist/plugins/hover.js';
import { Play, Pause, Download, Volume2, VolumeX } from 'lucide-react';
import { Card } from '../../shared/Card';
import { Button } from '../../shared/Button';
import { Skeleton } from '../../shared/Skeleton';
import { formatMs } from '../../lib/utils';
import { engine } from '../../engine';

interface AudioPlayerProps {
  filename: string;
  children?: React.ReactNode;
}

export function AudioPlayer({ filename, children }: AudioPlayerProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const wavesurferRef = useRef<WaveSurfer | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [isReady, setIsReady] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [isMuted, setIsMuted] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [downloaded, setDownloaded] = useState(false);

  useEffect(() => {
    if (!containerRef.current) return;

    const ctx = document.createElement('canvas').getContext('2d');
    let progressColor: string | CanvasGradient = '#FFA726'; // fallback
    if (ctx) {
      // Audio Wave Played Gradient (Orange to Yellow)
      const gradient = ctx.createLinearGradient(0, 0, 300, 0);
      gradient.addColorStop(0, '#FF8A65'); // Deep Orange
      gradient.addColorStop(1, '#FFCA28'); // Amber/Yellow
      progressColor = gradient;
    }

    const ws = WaveSurfer.create({
      container: containerRef.current,
      waveColor: '#FFE0B2', // Light orange for unplayed wave
      progressColor,
      cursorWidth: 2,
      cursorColor: '#FFCA28',
      height: 80,
      normalize: true,
      url: engine.getAudioUrl(filename),
      plugins: [
        Hover.create({
          lineColor: '#FFCA28',
          lineWidth: 1,
          labelBackground: 'rgba(0, 0, 0, 0.75)',
          labelColor: '#fff',
          labelSize: '11px',
        }),
      ],
    });

    wavesurferRef.current = ws;

    ws.on('ready', () => {
      setIsReady(true);
      setDuration(ws.getDuration());
    });

    ws.on('audioprocess', () => {
      setCurrentTime(ws.getCurrentTime());
    });

    ws.on('interaction', () => {
      setCurrentTime(ws.getCurrentTime());
    });

    ws.on('timeupdate', () => {
      setCurrentTime(ws.getCurrentTime());
    });

    ws.on('play', () => setIsPlaying(true));
    ws.on('pause', () => setIsPlaying(false));
    ws.on('finish', () => setIsPlaying(false));
    
    ws.on('error', (err) => {
      console.error('WaveSurfer error:', err);
      // Still try to hide the skeleton even if it fails, or show error state
      setIsReady(true);
    });

    // Abstract real engine loading is now handled via url in create()

    return () => {
      ws.destroy();
    };
  }, [filename]);

  const togglePlay = () => {
    if (wavesurferRef.current) {
      wavesurferRef.current.playPause();
    }
  };

  const toggleMute = () => {
    if (wavesurferRef.current) {
      const newMuted = !isMuted;
      wavesurferRef.current.setMuted(newMuted);
      setIsMuted(newMuted);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === ' ') {
      e.preventDefault();
      togglePlay();
    } else if (e.key === 'ArrowRight') {
      if (wavesurferRef.current) wavesurferRef.current.skip(5);
    } else if (e.key === 'ArrowLeft') {
      if (wavesurferRef.current) wavesurferRef.current.skip(-5);
    }
  };

  const handleDownload = async () => {
    setDownloading(true);
    try {
      const blob = await engine.fetchAudio(filename);
      const url = window.URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `mofa-podcast-${Date.now()}.wav`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      window.URL.revokeObjectURL(url);
      setDownloaded(true);
      setTimeout(() => setDownloaded(false), 2000);
    } catch (e) {
      console.error("Failed to download", e);
    } finally {
      setDownloading(false);
    }
  };

  return (
    <Card 
      className="p-6 border-black/10 bg-white shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-accent-cyan"
      tabIndex={0}
      onKeyDown={handleKeyDown}
      aria-label="Audio player"
    >
      <div className="relative mb-6">
        {!isReady && (
          <div className="absolute inset-0 flex items-center h-[80px]">
            <Skeleton className="w-full h-1/2 bg-black/5" />
          </div>
        )}
        <div ref={containerRef} className={`w-full ${!isReady ? 'opacity-0' : 'opacity-100 transition-opacity duration-500'}`} />
      </div>

      {children && <div className="mb-4">{children}</div>}

      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4">
          <button
            onClick={togglePlay}
            disabled={!isReady}
            className="w-12 h-12 rounded-full flex items-center justify-center bg-[linear-gradient(to_bottom_right,#FF6B6B,#FFD93D,#03A9F4)] text-white shadow-md hover:scale-105 active:scale-95 transition-all disabled:opacity-50 disabled:pointer-events-none"
            aria-label={isPlaying ? 'Pause' : 'Play'}
          >
            {isPlaying ? <Pause className="w-5 h-5 fill-current" /> : <Play className="w-5 h-5 fill-current ml-1" />}
          </button>
          
          <div className="font-mono text-sm text-text-dim">
            {formatMs(currentTime * 1000)} / {formatMs(duration * 1000)}
          </div>
        </div>

        <div className="flex items-center gap-2 text-text-dim">
          <Button variant="ghost" onClick={toggleMute} className="h-8 w-8 p-0 hover:text-black" disabled={!isReady}>
            {isMuted ? <VolumeX className="w-4 h-4" /> : <Volume2 className="w-4 h-4" />}
          </Button>
        </div>
      </div>
    </Card>
  );
}
