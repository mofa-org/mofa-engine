export interface ErrorDescriptor {
  message: string;
  actionLabel: string;
  actionCode: 'retry' | 'retryTts' | 'editScript';
  snippet?: string;
  title?: string;
  autoRecoverMs?: number;
}

export function getErrorDescriptor(capability: string, errorType?: string): ErrorDescriptor {
  if (errorType === 'NoCapableModel') {
    if (capability === 'Chat') {
      return {
        title: 'No Chat Model',
        message: 'No chat model available. Is Ollama running?',
        actionLabel: 'Back to start',
        actionCode: 'editScript',
        snippet: 'ollama serve'
      };
    } else {
      return {
        title: 'No TTS Model',
        message: 'No TTS model available. Is Kokoro running on port 8421?',
        actionLabel: 'Back to start',
        actionCode: 'editScript',
        snippet: 'docker run -p 8421:8421 ghcr.io/replicate/kokoro'
      };
    }
  }

  if (errorType === 'CircuitOpen') {
    return {
      title: 'Provider offline',
      message: 'The TTS provider is recovering. This usually resolves in ~30 seconds.',
      actionLabel: 'Retry now',
      actionCode: 'retryTts',
      autoRecoverMs: 4000
    };
  }

  if (errorType === 'Timeout') {
    return {
      title: 'Timeout',
      message: 'The model is taking longer than expected — it may still be loading.',
      actionLabel: 'Retry now',
      actionCode: 'retryTts',
      autoRecoverMs: 4000
    };
  }

  return {
    title: 'Error',
    message: 'An unexpected error occurred.',
    actionLabel: 'Try again',
    actionCode: 'retry',
  };
}
