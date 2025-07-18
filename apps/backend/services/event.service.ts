import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { Event, EventType, Task } from '@moatless/types'
import type { StorageBackend } from './storage.js'

export type EventHandler = (event: Event) => void | Promise<void>

export class EventService extends EventEmitter {
  private eventCache: Map<string, Event[]> = new Map()

  constructor(private storage: StorageBackend) {
    super()
    this.setMaxListeners(100)
  }

  async emitEvent(event: Event): Promise<void> {
    // Ensure event has required fields
    if (!event.eventId) {
      event.eventId = randomUUID()
    }
    if (!event.timestamp) {
      event.timestamp = new Date()
    }

    // Get cached events for this task or initialize
    if (!this.eventCache.has(event.taskId)) {
      const existingEvents = await this.storage.getEvents(event.taskId)
      this.eventCache.set(event.taskId, existingEvents)
    }

    // Append to cache
    const cachedEvents = this.eventCache.get(event.taskId)!
    cachedEvents.push(event)

    // Only save to disk if agent type is not claude-code
    if (event.agentType !== 'claude-code') {
      await this.storage.saveEvents(event.taskId, cachedEvents)
    }

    // Emit for real-time subscribers
    super.emit(`task:${event.taskId}`, event)

    // Also emit to global channel
    super.emit('global', event)
  }

  async getEvents(
    taskId: string,
    limit?: number,
    offset = 0,
    eventType?: EventType,
    task?: Task,
  ): Promise<Event[]> {
    let events: Event[] = []

    // Check cache first
    if (this.eventCache.has(taskId)) {
      events = this.eventCache.get(taskId)!
    } else {
      // Always read from our storage
      // Note: Claude message reading has been moved to MessageService
      events = await this.storage.getEvents(taskId)
      // Cache the loaded events
      // TODO: Only if running?
      this.eventCache.set(taskId, events)
    }

    // Filter by event type if specified
    if (eventType) {
      events = events.filter((event) => event.type === eventType)
    }

    // Apply pagination
    return events.slice(offset, limit ? offset + limit : undefined)
  }

  subscribe(taskId: string, handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch(console.error)
    }

    this.on(`task:${taskId}`, wrappedHandler)

    return () => {
      this.off(`task:${taskId}`, wrappedHandler)
    }
  }

  subscribeAll(handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch(console.error)
    }

    this.on('global', wrappedHandler)

    return () => {
      this.off('global', wrappedHandler)
    }
  }

  async getEventCount(taskId: string, eventType?: EventType, task?: Task): Promise<number> {
    const events = await this.getEvents(taskId, undefined, 0, eventType, task)
    return events.length
  }
}

export function createEventService(storage: StorageBackend): EventService {
  return new EventService(storage)
}
