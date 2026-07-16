import React, { useEffect, useState, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Search, Play, Activity, History, Trash2, Settings, Code } from 'lucide-react';
import { useHistory } from '../storage/useHistory';

interface Command {
  id: string;
  label: string;
  icon: React.ReactNode;
  action: () => void;
  shortcut?: string;
  keywords: string[];
}

export function CommandPalette() {
  const [isOpen, setIsOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const { clearHistory } = useHistory();

  const commands: Command[] = [
    {
      id: 'generate',
      label: 'Generate podcast',
      icon: <Play className="w-4 h-4 text-accent-cyan" />,
      action: () => { document.dispatchEvent(new KeyboardEvent('keydown', { metaKey: true, key: 'Enter' })); }, // Mock trigger
      shortcut: 'Cmd ↵',
      keywords: ['generate', 'start', 'podcast', 'make', 'create']
    },
    {
      id: 'monitor',
      label: 'Open Engine Monitor',
      icon: <Activity className="w-4 h-4 text-accent-green" />,
      action: () => document.dispatchEvent(new CustomEvent('open-monitor')),
      keywords: ['monitor', 'engine', 'status', 'telemetry', 'memory']
    },
    {
      id: 'history',
      label: 'Open History',
      icon: <History className="w-4 h-4 text-accent-purple" />,
      action: () => document.dispatchEvent(new CustomEvent('open-history')),
      keywords: ['history', 'recent', 'past']
    },
    {
      id: 'settings',
      label: 'Open Settings',
      icon: <Settings className="w-4 h-4 text-text-secondary" />,
      action: () => document.dispatchEvent(new CustomEvent('open-settings')),
      keywords: ['settings', 'preferences', 'config', 'url', 'voice']
    },
    {
      id: 'clear',
      label: 'Clear history',
      icon: <Trash2 className="w-4 h-4 text-accent-red" />,
      action: () => {
        if (confirm('Clear all history?')) clearHistory();
      },
      keywords: ['clear', 'delete', 'remove', 'history']
    },
    {
      id: 'github',
      label: 'View on GitHub',
      icon: <Code className="w-4 h-4 text-text-secondary" />,
      action: () => window.open('https://github.com/mofa-org/mofa-engine', '_blank'),
      keywords: ['github', 'repo', 'source', 'code']
    },
  ];

  const filteredCommands = query
    ? commands.filter(c => 
        c.label.toLowerCase().includes(query.toLowerCase()) || 
        c.keywords.some(k => k.includes(query.toLowerCase()))
      )
    : commands;

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        setIsOpen(prev => {
          if (!prev) {
            setQuery('');
            setSelectedIndex(0);
            setTimeout(() => inputRef.current?.focus(), 100);
          }
          return !prev;
        });
      }
      if (e.key === 'Escape' && isOpen) {
        e.preventDefault();
        setIsOpen(false);
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen]);

  const handleQueryChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setQuery(e.target.value);
    setSelectedIndex(0);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex(prev => Math.min(prev + 1, filteredCommands.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex(prev => Math.max(prev - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (filteredCommands[selectedIndex]) {
        filteredCommands[selectedIndex].action();
        setIsOpen(false);
      }
    }
  };

  return (
    <AnimatePresence>
      {isOpen && (
        <>
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={() => setIsOpen(false)}
            className="fixed inset-0 bg-background-primary/60 backdrop-blur-sm z-[100]"
          />
          <div className="fixed inset-0 z-[101] flex items-start justify-center pt-[15vh] px-4 pointer-events-none">
            <motion.div
              role="dialog"
              aria-modal="true"
              aria-label="Command Palette"
              initial={{ opacity: 0, scale: 0.95, y: -20 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.95, y: -20 }}
              transition={{ duration: 0.2, ease: 'easeOut' }}
              className="w-full max-w-xl bg-background-secondary border border-black/10 rounded-xl shadow-xl overflow-hidden pointer-events-auto flex flex-col max-h-[60vh]"
            >
              <div className="flex items-center px-4 border-b border-black/10 shrink-0">
                <Search className="w-5 h-5 text-text-dim shrink-0" />
                <input
                  ref={inputRef}
                  value={query}
                  onChange={handleQueryChange}
                  onKeyDown={handleKeyDown}
                  placeholder="Type a command or search..."
                  className="flex-1 bg-transparent border-none outline-none px-4 py-4 text-[15px] text-text-primary placeholder:text-text-dim"
                  aria-label="Command palette input"
                />
                <span className="text-[10px] font-mono text-text-dim px-2 py-1 rounded bg-black/5 shrink-0">ESC</span>
              </div>
              
              <div className="flex-1 overflow-y-auto py-2">
                {filteredCommands.length === 0 ? (
                  <div className="px-4 py-8 text-center text-[13px] text-text-dim">
                    No commands found.
                  </div>
                ) : (
                  filteredCommands.map((cmd, i) => (
                    <div
                      key={cmd.id}
                      onClick={() => {
                        cmd.action();
                        setIsOpen(false);
                      }}
                      onMouseEnter={() => setSelectedIndex(i)}
                      className={`flex items-center justify-between px-4 py-3 cursor-pointer ${
                        i === selectedIndex ? 'bg-black/5' : 'hover:bg-black/5'
                      }`}
                      role="button"
                      tabIndex={-1}
                    >
                      <div className="flex items-center gap-3">
                        {cmd.icon}
                        <span className={`text-[14px] ${i === selectedIndex ? 'text-text-primary' : 'text-text-secondary'}`}>
                          {cmd.label}
                        </span>
                      </div>
                      {cmd.shortcut && (
                        <span className="text-[11px] font-mono text-text-dim tracking-wider">
                          {cmd.shortcut}
                        </span>
                      )}
                    </div>
                  ))
                )}
              </div>
            </motion.div>
          </div>
        </>
      )}
    </AnimatePresence>
  );
}
