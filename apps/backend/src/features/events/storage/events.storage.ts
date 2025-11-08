import type { Event } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Storage } from '../../../infrastructure/storage/storage'
import { InfrastructureStorageSymbol } from '../../../infrastructure/storage/storage'

/**
 * Storage adapter for event persistence.
 * Uses the infrastructure Storage interface to persist events as JSON arrays.
 * Events for each task are stored at key: events/{taskId}
 */
@injectable()
export class EventsStorage {
  constructor(@inject(InfrastructureStorageSymbol) private readonly storage: Storage) {}

  /**
   * Save events for a task
   * @param taskId - Task identifier
   * @param events - Array of events to save
   */
  async saveEvents(taskId: string, events: Event[]): Promise<void> {
    await this.storage.save(this.getKey(taskId), events)
  }

  /**
   * Get all events for a task
   * @param taskId - Task identifier
   * @returns Array of events (empty array if none exist)
   */
  async getEvents(taskId: string): Promise<Event[]> {
    const events = await this.storage.get<Event[]>(this.getKey(taskId))
    return events || []
  }

  /**
   * Delete all events for a task
   * @param taskId - Task identifier
   */
  async deleteEvents(taskId: string): Promise<void> {
    await this.storage.delete(this.getKey(taskId))
  }

  /**
   * Generate storage key for task events
   * @param taskId - Task identifier
   * @returns Storage key
   */
  private getKey(taskId: string): string {
    return `events/${taskId}`
  }
}
