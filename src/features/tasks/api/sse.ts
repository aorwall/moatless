import type { Event } from '@/types/event.ts';

import { NDJSONSSEClient } from './ndjson-sse';

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
  private client: NDJSONSSEClient;

  constructor(options: SSEClientOptions) {
    // Delegate to NDJSON client which handles application/x-ndjson MIME type
    this.client = new NDJSONSSEClient(options);
  }

  connect(): void {
    this.client.connect();
  }

  disconnect(): void {
    this.client.disconnect();
  }

  resetConnection(): void {
    this.client.resetConnection();
  }

  isConnected(): boolean {
    return this.client.isConnected();
  }

  getReadyState(): number {
    // For compatibility, return EventSource-like states
    return this.client.isConnected() ? 1 : 2; // 1 = OPEN, 2 = CLOSED
  }
}
