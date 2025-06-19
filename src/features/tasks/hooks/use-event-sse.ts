import { useCallback, useEffect, useRef, useState } from 'react';

import type { AgentEvent } from '@/types/agent-events';

const API_BASE_URL = import.meta.env.VITE_API_URL || 'http://localhost:8000';

export interface UseEventSSEOptions {
  taskId: string;
  enabled?: boolean;
  onEvent?: (event: AgentEvent) => void;
  onError?: (error: Error) => void;
  onConnect?: () => void;
  onDisconnect?: () => void;
}

export function useEventSSE({
  taskId,
  enabled = true,
  onEvent,
  onError,
  onConnect,
  onDisconnect,
}: UseEventSSEOptions) {
  const [isConnected, setIsConnected] = useState(false);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const reconnectTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const lastEventIdRef = useRef<string | null>(null);

  const disconnect = useCallback(() => {
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current);
      reconnectTimeoutRef.current = null;
    }

    if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
      setIsConnected(false);
      onDisconnect?.();
    }
  }, [onDisconnect]);

  const connect = useCallback(() => {
    if (!enabled || eventSourceRef.current) {
      return;
    }

    try {
      // Build URL with optional last event ID for resuming
      let url = `${API_BASE_URL}/api/v1/tasks/${taskId}/stream`;
      if (lastEventIdRef.current) {
        url += `?last_event_id=${encodeURIComponent(lastEventIdRef.current)}`;
      }

      const eventSource = new EventSource(url, {
        withCredentials: true,
      });
      eventSourceRef.current = eventSource;

      eventSource.onopen = () => {
        setIsConnected(true);
        setConnectionError(null);
        onConnect?.();
      };

      eventSource.addEventListener('message', (event) => {
        try {
          const eventData = JSON.parse(event.data) as AgentEvent;

          // Store the last event ID for reconnection
          if (eventData.event_id) {
            lastEventIdRef.current = eventData.event_id;
          }

          onEvent?.(eventData);
        } catch (error) {
          console.error('Failed to parse event:', error);
          onError?.(new Error('Failed to parse event data'));
        }
      });

      // Listen for specific event types
      const eventTypes = [
        'environment',
        'workspace',
        'input',
        'message',
        'tool_call',
        'tool_result',
        'result',
      ];

      eventTypes.forEach((eventType) => {
        eventSource.addEventListener(eventType, (event) => {
          try {
            const eventData = JSON.parse(event.data) as AgentEvent;

            // Store the last event ID
            if (eventData.event_id) {
              lastEventIdRef.current = eventData.event_id;
            }

            onEvent?.(eventData);
          } catch (error) {
            console.error(`Failed to parse ${eventType} event:`, error);
            onError?.(new Error(`Failed to parse ${eventType} event`));
          }
        });
      });

      eventSource.onerror = (error) => {
        console.error('EventSource error:', error);
        setConnectionError('Connection error');
        setIsConnected(false);
        disconnect();

        // Attempt to reconnect after a delay if enabled
        if (enabled) {
          reconnectTimeoutRef.current = setTimeout(() => {
            connect();
          }, 5000);
        }

        onError?.(new Error('EventSource connection error'));
      };
    } catch (error) {
      console.error('Failed to create EventSource:', error);
      setConnectionError('Failed to create connection');
      onError?.(error instanceof Error ? error : new Error('Unknown error'));
    }
  }, [enabled, taskId, disconnect, onConnect, onEvent, onError]);

  useEffect(() => {
    if (enabled) {
      connect();
    } else {
      disconnect();
    }

    return () => {
      disconnect();
    };
  }, [enabled, connect, disconnect]);

  return {
    isConnected,
    connectionError,
    connect,
    disconnect,
  };
}
