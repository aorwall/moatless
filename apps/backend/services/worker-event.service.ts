import type { Event } from '@moatless/shared/types/event'
import { createLogger } from '../utils/logger.js'

/**
 * WorkerEventService - Event service implementation for Docker worker mode
 *
 * This service is used by Docker workers to forward events to the main backend
 * through task-specific endpoints. It implements the EventService interface but
 * only supports event emission, as workers don't need subscription or retrieval.
 *
 * Workers use this service to:
 * - Forward agent execution events to the main backend
 * - Maintain event flow from isolated Docker containers
 * - Ensure all events are properly associated with their task
 */
export class WorkerEventService {
  private logger = createLogger()

  constructor(
    private apiUrl: string,
    private apiKey?: string,
    private taskId?: string,
  ) {}

  /**
   * Emit an event to the main backend through the worker event endpoint
   * Events are forwarded to /api/v1/tasks/{taskId}/worker/events
   */
  async emitEvent(event: Event): Promise<void> {
    try {
      // Use task-specific endpoint - taskId is required for worker events
      const taskId = this.taskId || event.taskId
      if (!taskId) {
        throw new Error('TaskId is required for worker event emission')
      }

      const response = await fetch(`${this.apiUrl}/api/v1/tasks/${taskId}/worker/events`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(this.apiKey && { Authorization: `Bearer ${this.apiKey}` }),
        },
        body: JSON.stringify(event),
      })

      if (!response.ok) {
        throw new Error(`Failed to forward event to main backend: ${response.statusText}`)
      }
    } catch (error) {
      this.logger.error('Worker failed to forward event', error as Error, {
        eventType: event.type,
        taskId: event.taskId,
        workerMode: true,
      })
      throw error
    }
  }

  // EventService methods not implemented in worker mode
  // Workers only emit events, they don't subscribe or retrieve
  subscribeToTaskEvents(_taskId: string): AsyncGenerator<Event> {
    throw new Error('Event subscription not available in worker mode - workers only emit events')
  }

  getAllTaskEvents(_taskId: string): Promise<Event[]> {
    throw new Error('Event retrieval not available in worker mode - workers only emit events')
  }

  clearTaskEvents(_taskId: string): Promise<void> {
    throw new Error('Event clearing not available in worker mode - workers only emit events')
  }
}
