import type { Event } from '@/types/event.ts';

const API_BASE_URL = import.meta.env.VITE_API_URL || 'http://localhost:8000';

export interface SSEClientOptions {
  taskId: string;
  apiUrl?: string;
  onEvent?: (event: Event) => void;
  onError?: (error: Error) => void;
  onConnect?: () => void;
  onDisconnect?: () => void;
  onTaskNotFound?: () => void;
}

export class TaskSSEClient {
  private eventSource: EventSource | null = null;
  private taskId: string;
  private apiUrl: string;
  private onEvent?: (event: Event) => void;
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
  private healthCheckInterval: NodeJS.Timeout | null = null;

  constructor(options: SSEClientOptions) {
    this.taskId = options.taskId;
    this.apiUrl = options.apiUrl || API_BASE_URL;
    this.onEvent = options.onEvent;
    this.onError = options.onError;
    this.onConnect = options.onConnect;
    this.onDisconnect = options.onDisconnect;
    this.onTaskNotFound = options.onTaskNotFound;
  }

  connect(): void {
    if (this.eventSource || this.isConnecting || this.taskNotFound) {
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

      // Add a check for immediate closure
      setTimeout(() => {
        if (
          this.eventSource &&
          this.eventSource.readyState === EventSource.CLOSED
        ) {
          // EventSource closed immediately - this is normal
        }
      }, 100);

      this.eventSource.onopen = () => {
        this.isConnecting = false;
        this.reconnectAttempts = 0; // Reset attempts on successful connection
        this.onConnect?.();

        // Clear any pending reconnect
        if (this.reconnectTimeout) {
          clearTimeout(this.reconnectTimeout);
          this.reconnectTimeout = null;
        }

        // Start health check
        this.startHealthCheck();
      };

      // Note: onmessage handler removed as we only use named events now

      // Handle custom events
      this.eventSource.addEventListener('ping', () => {
        // Ping received - connection is alive
      });

      // Handle agent events
      this.eventSource.addEventListener('event', (event) => {
        try {
          const agentEvent = JSON.parse(event.data) as Event;
          this.onEvent?.(agentEvent);
        } catch (error) {
          const parseError = new Error(
            `Failed to parse agent event: ${error instanceof Error ? error.message : String(error)}`,
          );
          this.onError?.(parseError);
        }
      });

      this.eventSource.addEventListener('error', (_event) => {
        // Error event received - will be handled by onerror
        // Don't call onError here to avoid duplicate error messages
      });

      this.eventSource.onerror = async (_event) => {
        await this.handleConnectionError();
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

    this.stopHealthCheck();
  }

  resetConnection(): void {
    // Reset connection state, useful when retrying after errors
    this.taskNotFound = false;
    this.shouldReconnect = true;
    this.reconnectAttempts = 0;
  }

  private async handleConnectionError(): Promise<void> {
    this.isConnecting = false;

    // Store current shouldReconnect state before disconnect
    const shouldAttemptReconnect = this.shouldReconnect && !this.taskNotFound;

    // Close the current connection but don't prevent reconnection yet
    this.closeCurrentConnection();

    if (shouldAttemptReconnect) {
      await this.handleReconnectionLogic();
    } else if (!this.shouldReconnect) {
      // Manual disconnect or task not found
      this.onDisconnect?.();
    }
  }

  private closeCurrentConnection(): void {
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
    }
    this.stopHealthCheck();
  }

  private async handleReconnectionLogic(): Promise<void> {
    // On first error or after a few attempts, check if task exists
    if (this.reconnectAttempts === 0 || this.reconnectAttempts === 2) {
      const taskExists = await this.checkTaskExists();
      if (!taskExists) {
        return;
      }
    }

    this.attemptReconnection();
  }

  private async checkTaskExists(): Promise<boolean> {
    try {
      const response = await fetch(
        `${this.apiUrl}/api/v1/tasks/${this.taskId}`,
        {
          method: 'HEAD',
          credentials: 'include',
        },
      );
      if (response.status === 404) {
        this.handleTaskNotFound();
        return false;
      }
    } catch {
      // If we can't check, proceed with normal reconnection
      // Network error checking task existence - will attempt normal reconnection
    }
    return true;
  }

  private handleTaskNotFound(): void {
    this.taskNotFound = true;
    this.shouldReconnect = false;
    this.onTaskNotFound?.();
    this.onError?.(new Error('Task not found (404)'));
    this.onDisconnect?.();
  }

  private attemptReconnection(): void {
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

  private startHealthCheck(): void {
    this.stopHealthCheck();

    this.healthCheckInterval = setInterval(() => {
      if (this.eventSource) {
        const state = this.eventSource.readyState;

        // If the connection is closed unexpectedly, handle it
        if (state === EventSource.CLOSED && this.shouldReconnect) {
          this.handleConnectionError();
        }
      }
    }, 10000); // Check every 10 seconds
  }

  private stopHealthCheck(): void {
    if (this.healthCheckInterval) {
      clearInterval(this.healthCheckInterval);
      this.healthCheckInterval = null;
    }
  }
}
