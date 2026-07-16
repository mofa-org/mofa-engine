import { useState } from 'react';

interface Settings {
  soundEnabled: boolean;
  reducedMotion: boolean;
  defaultVoice: string;
}

const DEFAULT_SETTINGS: Settings = {
  soundEnabled: true,
  reducedMotion: false,
  defaultVoice: 'Nova',
};

export function useSettings() {
  const [settings, setSettings] = useState<Settings>(() => {
    const raw = localStorage.getItem('mofa_settings');
    if (raw) {
      try {
        return { ...DEFAULT_SETTINGS, ...JSON.parse(raw) };
      } catch {
        return DEFAULT_SETTINGS;
      }
    }
    return DEFAULT_SETTINGS;
  });

  const updateSettings = (updates: Partial<Settings>) => {
    setSettings((prev) => {
      const next = { ...prev, ...updates };
      localStorage.setItem('mofa_settings', JSON.stringify(next));
      return next;
    });
  };

  return { settings, updateSettings };
}
