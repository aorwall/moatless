import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useRef, useState } from 'react';

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

  // Store callbacks in refs to avoid recreating SSE client
  const onMessageRef = useRef(onMessage);
  const onErrorRef = useRef(onError);

  // Update refs when callbacks change
  useEffect(() => {
    onMessageRef.current = onMessage;
  }, [onMessage]);

  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  // Stable callbacks that use refs
  const handleMessage = useCallback((message: BaseMessage) => {
    setLastMessage(message);
    setError(null);
    onMessageRef.current?.(message);
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
      queryClient,
      onMessage: handleMessage,
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
    queryClient,
    handleMessage,
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
    lastMessage,
    error,
  };
}
