import type { QueryClient } from '@tanstack/react-query';

import type { BaseMessage } from '@/types/task';

const API_BASE_URL = import.meta.env.VITE_API_URL || 'http://localhost:8000';

export interface SSEClientOptions {
  taskId: string;
  apiUrl?: string;
  queryClient?: QueryClient;
  onMessage?: (message: BaseMessage) => void;
  onError?: (error: Error) => void;
  onConnect?: () => void;
  onDisconnect?: () => void;
  onTaskNotFound?: () => void;
}

export class TaskSSEClient {
  private eventSource: EventSource | null = null;
  private taskId: string;
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
  private taskNotFound = false;
  private onTaskNotFound?: () => void;

  constructor(options: SSEClientOptions) {
    this.taskId = options.taskId;
    this.apiUrl = options.apiUrl || API_BASE_URL;
    this.queryClient = options.queryClient;
    this.onMessage = options.onMessage;
    this.onError = options.onError;
    this.onConnect = options.onConnect;
    this.onDisconnect = options.onDisconnect;
    this.onTaskNotFound = options.onTaskNotFound;
  }

  connect(): void {
    if (this.eventSource || this.isConnecting || this.taskNotFound) {
      // SSE already connected, connecting, or task not found - silently return
      return;
    }

    this.isConnecting = true;
    this.shouldReconnect = true; // Reset on new connection attempt

    // Build URL with optional last event ID for resuming
    let url = `${this.apiUrl}/api/v1/tasks/${this.taskId}/stream`;
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
            // Invalidate task messages query
            this.queryClient.invalidateQueries({
              queryKey: ['task-messages', this.taskId],
            });

            // Invalidate task details
            this.queryClient.invalidateQueries({
              queryKey: ['task', this.taskId],
            });

            // Invalidate tasks list if status changed
            if (message.type === 'result') {
              this.queryClient.invalidateQueries({
                queryKey: ['tasks'],
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
        // Error event received - will be handled by onerror
        // Don't call onError here to avoid duplicate error messages
      });

      this.eventSource.onerror = async () => {
        this.isConnecting = false;

        // Store current shouldReconnect state before disconnect
        const shouldAttemptReconnect =
          this.shouldReconnect && !this.taskNotFound;

        // Close the current connection but don't prevent reconnection yet
        if (this.eventSource) {
          this.eventSource.close();
          this.eventSource = null;
        }

        // Check if task exists before attempting reconnection
        if (shouldAttemptReconnect) {
          // On first error or after a few attempts, check if task exists
          if (this.reconnectAttempts === 0 || this.reconnectAttempts === 2) {
            try {
              const response = await fetch(
                `${this.apiUrl}/api/v1/tasks/${this.taskId}`,
                {
                  method: 'HEAD',
                  credentials: 'include',
                },
              );
              if (response.status === 404) {
                this.taskNotFound = true;
                this.shouldReconnect = false;
                this.onTaskNotFound?.();
                this.onError?.(new Error('Task not found (404)'));
                this.onDisconnect?.();
                return;
              }
            } catch {
              // If we can't check, proceed with normal reconnection
              // Network error checking task existence - will attempt normal reconnection
            }
          }

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
        } else if (!this.shouldReconnect) {
          // Manual disconnect or task not found
          this.onDisconnect?.();
        }
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

  resetConnection(): void {
    // Reset connection state, useful when retrying after errors
    this.taskNotFound = false;
    this.shouldReconnect = true;
    this.reconnectAttempts = 0;
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
    const delay = Math.max(baseDelay + jitter, 1000); // Ensure minimum 1 second delay

    // Scheduling SSE reconnection with backoff (attempt ${this.reconnectAttempts}/${this.maxReconnectAttempts})
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
