import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';

import { TaskSSEClient } from '@/lib/api/sse';
import type { BaseMessage } from '@/types/task';

export interface UseTaskSSEOptions {
  taskId: string;
  apiUrl?: string;
  onMessage?: (message: BaseMessage) => void;
  onError?: (error: Error) => void;
  autoConnect?: boolean;
}

export interface UseTaskSSEReturn {
  isConnected: boolean;
  connect: () => void;
  disconnect: () => void;
  lastMessage: BaseMessage | null;
  error: Error | null;
}

export function useTaskSSE({
  taskId,
  apiUrl,
  onMessage,
  onError,
  autoConnect = true,
}: UseTaskSSEOptions): UseTaskSSEReturn {
  const queryClient = useQueryClient();
  const sseClientRef = useRef<TaskSSEClient | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [lastMessage, setLastMessage] = useState<BaseMessage | null>(null);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    // Create SSE client
    const client = new TaskSSEClient({
      taskId,
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
      onTaskNotFound: () => {
        setError(new Error('Task not found'));
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
  }, [taskId, apiUrl, queryClient, autoConnect, onMessage, onError]);

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
