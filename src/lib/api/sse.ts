import type { QueryClient } from '@tanstack/react-query';

import type { BaseMessage } from '@/types/session';

const API_BASE_URL = import.meta.env.VITE_API_URL || 'http://localhost:8000';

export interface SSEClientOptions {
  sessionId: string;
  apiUrl?: string;
  queryClient?: QueryClient;
  onMessage?: (message: BaseMessage) => void;
  onError?: (error: Error) => void;
  onConnect?: () => void;
  onDisconnect?: () => void;
}

export class SessionSSEClient {
  private eventSource: EventSource | null = null;
  private sessionId: string;
  private apiUrl: string;
  private queryClient?: QueryClient;
  private onMessage?: (message: BaseMessage) => void;
  private onError?: (error: Error) => void;
  private onConnect?: () => void;
  private onDisconnect?: () => void;
  private reconnectTimeout: NodeJS.Timeout | null = null;
  private lastEventId: string | null = null;
  private isConnecting = false;
  private reconnectAttempts = 0;
  private maxReconnectAttempts = 10;
  private reconnectDelay = 5000;
  private shouldReconnect = true;

  constructor(options: SSEClientOptions) {
    this.sessionId = options.sessionId;
    this.apiUrl = options.apiUrl || API_BASE_URL;
    this.queryClient = options.queryClient;
    this.onMessage = options.onMessage;
    this.onError = options.onError;
    this.onConnect = options.onConnect;
    this.onDisconnect = options.onDisconnect;
  }

  connect(): void {
    if (this.eventSource || this.isConnecting) {
      // SSE already connected or connecting - silently return
      return;
    }

    this.isConnecting = true;

    // Build URL with optional last event ID for resuming
    let url = `${this.apiUrl}/api/v1/sessions/${this.sessionId}/stream`;
    if (this.lastEventId) {
      url += `?last_event_id=${encodeURIComponent(this.lastEventId)}`;
    }

    try {
      this.eventSource = new EventSource(url, {
        withCredentials: true,
      });

      this.eventSource.onopen = () => {
        this.isConnecting = false;
        this.reconnectAttempts = 0; // Reset attempts on successful connection
        this.onConnect?.();

        // Clear any pending reconnect
        if (this.reconnectTimeout) {
          clearTimeout(this.reconnectTimeout);
          this.reconnectTimeout = null;
        }
      };

      this.eventSource.onmessage = (event) => {
        // Store the last event ID for reconnection
        if (event.lastEventId) {
          this.lastEventId = event.lastEventId;
        }

        try {
          const message = JSON.parse(event.data) as BaseMessage;
          // Message received - will be handled by callbacks

          // Update React Query cache
          if (this.queryClient) {
            // Invalidate session messages query
            this.queryClient.invalidateQueries({
              queryKey: ['session-messages', this.sessionId],
            });

            // Invalidate session details
            this.queryClient.invalidateQueries({
              queryKey: ['session', this.sessionId],
            });

            // Invalidate sessions list if status changed
            if (message.type === 'result') {
              this.queryClient.invalidateQueries({
                queryKey: ['sessions'],
              });
            }
          }

          this.onMessage?.(message);
        } catch (error) {
          const parseError = new Error(
            `Failed to parse SSE message: ${error instanceof Error ? error.message : String(error)}`,
          );
          this.onError?.(parseError);
        }
      };

      // Handle custom events
      this.eventSource.addEventListener('ping', () => {
        // Ping received - connection is alive
      });

      this.eventSource.addEventListener('error', () => {
        this.onError?.(new Error('SSE error event received'));
      });

      this.eventSource.onerror = () => {
        this.isConnecting = false;
        // SSE connection error - will attempt to reconnect

        // EventSource automatically reconnects, but we handle it manually
        this.disconnect();

        // Only reconnect if not manually disconnected and within retry limit
        if (
          this.shouldReconnect &&
          this.reconnectAttempts < this.maxReconnectAttempts
        ) {
          this.scheduleReconnect();
        } else if (this.reconnectAttempts >= this.maxReconnectAttempts) {
          this.onError?.(
            new Error(
              `SSE connection failed after ${this.maxReconnectAttempts} attempts`,
            ),
          );
        }

        this.onError?.(new Error('SSE connection error'));
      };
    } catch (error) {
      this.isConnecting = false;
      const connectionError = new Error(
        `Failed to create EventSource: ${error instanceof Error ? error.message : String(error)}`,
      );
      this.onError?.(connectionError);
      this.scheduleReconnect();
    }
  }

  disconnect(): void {
    this.shouldReconnect = false; // Prevent reconnection on manual disconnect

    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = null;
    }

    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
      this.onDisconnect?.();
    }
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimeout) {
      return;
    }

    this.reconnectAttempts++;

    // Calculate exponential backoff with jitter
    const baseDelay = Math.min(
      this.reconnectDelay * 2 ** (this.reconnectAttempts - 1),
      30000,
    );
    const jitter = Math.random() * 1000;
    const delay = baseDelay + jitter;

    // Scheduling SSE reconnection with backoff
    this.reconnectTimeout = setTimeout(() => {
      this.reconnectTimeout = null;
      this.connect();
    }, delay);
  }

  isConnected(): boolean {
    return this.eventSource?.readyState === EventSource.OPEN;
  }

  getReadyState(): number {
    return this.eventSource?.readyState ?? EventSource.CLOSED;
  }
}
