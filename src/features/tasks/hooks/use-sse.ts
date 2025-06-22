import { useCallback, useEffect, useRef, useState } from 'react';

import { TaskSSEClient } from '@/features/tasks/api/sse.ts';
import type { Event } from '@/types/event.ts';

export interface UseTaskSSEOptions {
  taskId: string;
  apiUrl?: string;
  onEvent?: (event: Event) => void;
  onError?: (error: Error) => void;
  autoConnect?: boolean;
}

export interface UseTaskSSEReturn {
  isConnected: boolean;
  connect: () => void;
  disconnect: () => void;
  error: Error | null;
}

export function useTaskSSE({
  taskId,
  apiUrl,
  onEvent,
  onError,
  autoConnect = true,
}: UseTaskSSEOptions): UseTaskSSEReturn {
  const sseClientRef = useRef<TaskSSEClient | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<Error | null>(null);

  // Store callbacks in refs to avoid recreating SSE client
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);

  // Update refs when callbacks change
  useEffect(() => {
    onEventRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  // Stable callbacks that use refs
  const handleEvent = useCallback((event: Event) => {
    setError(null);
    onEventRef.current?.(event);
  }, []);

  const handleError = useCallback((err: Error) => {
    setError(err);
    onErrorRef.current?.(err);
  }, []);

  const handleConnect = useCallback(() => {
    setIsConnected(true);
    setError(null);
  }, []);

  const handleDisconnect = useCallback(() => {
    setIsConnected(false);
  }, []);

  const handleTaskNotFound = useCallback(() => {
    setError(new Error('Task not found'));
    setIsConnected(false);
  }, []);

  // Create SSE client only when taskId or apiUrl changes
  useEffect(() => {
    // Create SSE client
    const client = new TaskSSEClient({
      taskId,
      apiUrl,
      onEvent: handleEvent,
      onError: handleError,
      onConnect: handleConnect,
      onDisconnect: handleDisconnect,
      onTaskNotFound: handleTaskNotFound,
    });

    sseClientRef.current = client;

    // Cleanup on unmount
    return () => {
      client.disconnect();
      sseClientRef.current = null;
    };
  }, [
    taskId,
    apiUrl,
    handleEvent,
    handleError,
    handleConnect,
    handleDisconnect,
    handleTaskNotFound,
  ]);

  // Handle autoConnect changes separately
  useEffect(() => {
    if (sseClientRef.current) {
      if (autoConnect) {
        sseClientRef.current.connect();
      } else {
        sseClientRef.current.disconnect();
      }
    }
  }, [autoConnect]);

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
    error,
  };
}
