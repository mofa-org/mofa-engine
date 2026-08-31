import { useQuery } from '@tanstack/react-query';
import { engine, useEngineUrl } from './index';

export type ConnectionState = 'connecting' | 'connected' | 'disconnected';

export function useEngineConnection() {
  const url = useEngineUrl();
  const { data, error, isFetching } = useQuery({
    queryKey: ['engine_health', url],
    queryFn: () => engine.getHealth(),
    refetchInterval: 3000,
    retry: false,
  });

  let state: ConnectionState = 'connecting';
  if (data?.success) {
    state = 'connected';
  } else if (data?.success === false || error) {
    state = 'disconnected';
  }

  return {
    state,
    version: data?.success ? data.data.version : null,
    uptime_secs: data?.success ? data.data.uptime_secs : null,
    isFetching
  };
}
