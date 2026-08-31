import { useState, useEffect } from 'react';

export function useElapsed(startedAt: number | null, isRunning: boolean) {
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    if (!startedAt || !isRunning) {
      return;
    }

    const interval = setInterval(() => {
      setElapsed(Date.now() - startedAt);
    }, 100);

    return () => clearInterval(interval);
  }, [startedAt, isRunning]);

  return elapsed;
}
