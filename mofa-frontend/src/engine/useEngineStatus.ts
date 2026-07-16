import { useQuery } from '@tanstack/react-query';
import { engine, useEngineUrl } from './index';
import { useEngineConnection } from './useEngineConnection';

export function useEngineStatus() {
  const { state } = useEngineConnection();
  const url = useEngineUrl();

  return useQuery({
    queryKey: ['engine_status', url],
    queryFn: () => engine.getStatus(),
    refetchInterval: 2000,
    enabled: state === 'connected',
  });
}
