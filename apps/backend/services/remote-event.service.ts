import type { Event } from '@moatless/shared/types/event'
import { createLogger } from '../utils/logger.js'

export class RemoteEventService {
  private logger = createLogger()

  constructor(
    private apiUrl: string,
    private apiKey: string,
  ) {}

  async emitEvent(event: Event): Promise<void> {
    try {
      const response = await fetch(`${this.apiUrl}/api/v1/worker/events`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${this.apiKey}`,
        },
        body: JSON.stringify(event),
      })

      if (!response.ok) {
        throw new Error(`Failed to forward event: ${response.statusText}`)
      }
    } catch (error) {
      this.logger.error('Failed to forward event', error as Error, {
        eventType: event.type,
        taskId: event.taskId,
      })
      throw error
    }
  }

  // Other EventService methods that aren't needed in worker mode
  subscribeToTaskEvents(_taskId: string): AsyncGenerator<Event> {
    throw new Error('Event subscription not available in worker mode')
  }

  getAllTaskEvents(_taskId: string): Promise<Event[]> {
    throw new Error('Event retrieval not available in worker mode')
  }

  clearTaskEvents(_taskId: string): Promise<void> {
    throw new Error('Event clearing not available in worker mode')
  }
}
