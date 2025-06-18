import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';

import { SessionSSEClient } from '@/lib/api/sse';
import type { BaseMessage } from '@/types/session';

export interface UseSessionSSEOptions {
  sessionId: string;
  apiUrl?: string;
  onMessage?: (message: BaseMessage) => void;
  onError?: (error: Error) => void;
  autoConnect?: boolean;
}

export interface UseSessionSSEReturn {
  isConnected: boolean;
  connect: () => void;
  disconnect: () => void;
  lastMessage: BaseMessage | null;
  error: Error | null;
}

export function useSessionSSE({
  sessionId,
  apiUrl,
  onMessage,
  onError,
  autoConnect = true,
}: UseSessionSSEOptions): UseSessionSSEReturn {
  const queryClient = useQueryClient();
  const sseClientRef = useRef<SessionSSEClient | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [lastMessage, setLastMessage] = useState<BaseMessage | null>(null);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    // Create SSE client
    const client = new SessionSSEClient({
      sessionId,
      apiUrl,
      queryClient,
      onMessage: (message) => {
        setLastMessage(message);
        setError(null);
        onMessage?.(message);
      },
      onError: (err) => {
        setError(err);
        onError?.(err);
      },
      onConnect: () => {
        setIsConnected(true);
        setError(null);
      },
      onDisconnect: () => {
        setIsConnected(false);
      },
    });

    sseClientRef.current = client;

    // Auto-connect if enabled
    if (autoConnect) {
      client.connect();
    }

    // Cleanup on unmount
    return () => {
      client.disconnect();
      sseClientRef.current = null;
    };
  }, [sessionId, apiUrl, queryClient, autoConnect, onMessage, onError]);

  const connect = () => {
    sseClientRef.current?.connect();
  };

  const disconnect = () => {
    sseClientRef.current?.disconnect();
  };

  return {
    isConnected,
    connect,
    disconnect,
    lastMessage,
    error,
  };
}
