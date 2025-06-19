import { useCallback, useEffect, useRef, useState } from 'react';

import { TaskSSEClient } from '@/lib/api/sse';
import type { AgentEvent } from '@/types/agent-events';
import type { BaseMessage } from '@/types/task';

export interface UseTaskSSEOptions {
  taskId: string;
  apiUrl?: string;
  onMessage?: (message: BaseMessage) => void;
  onEvent?: (event: AgentEvent) => void;
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
  onEvent,
  onError,
  autoConnect = true,
}: UseTaskSSEOptions): UseTaskSSEReturn {
  const sseClientRef = useRef<TaskSSEClient | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [lastMessage, setLastMessage] = useState<BaseMessage | null>(null);
  const [error, setError] = useState<Error | null>(null);

  // Store callbacks in refs to avoid recreating SSE client
  const onMessageRef = useRef(onMessage);
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);

  // Update refs when callbacks change
  useEffect(() => {
    onMessageRef.current = onMessage;
  }, [onMessage]);

  useEffect(() => {
    onEventRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  // Stable callbacks that use refs
  const handleMessage = useCallback((message: BaseMessage) => {
    setLastMessage(message);
    setError(null);
    onMessageRef.current?.(message);
  }, []);

  const handleEvent = useCallback((event: AgentEvent) => {
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
      onMessage: handleMessage,
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
    handleMessage,
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
    lastMessage,
    error,
  };
}
