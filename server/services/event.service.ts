import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { FastifyInstance } from 'fastify'
import type { Event, EventType } from '../../shared/types/index.js'
import type { StorageBackend } from './storage.js'

export type EventHandler = (event: Event) => void | Promise<void>

export class EventService extends EventEmitter {
  constructor(private storage: StorageBackend) {
    super()
    this.setMaxListeners(100) // Increase limit for multiple subscriptions
  }

  async emitEvent(event: Event): Promise<void> {
    // Ensure event has required fields
    if (!event.eventId) {
      event.eventId = randomUUID()
    }
    if (!event.timestamp) {
      event.timestamp = new Date()
    }

    // Store event to file (similar to file_stream.py)
    const existingEvents = await this.storage.getEvents(event.taskId)
    existingEvents.push(event)
    await this.storage.saveEvents(event.taskId, existingEvents)

    // Emit for real-time subscribers
    super.emit(`task:${event.taskId}`, event)
    super.emit('event', event)
    super.emit(`type:${event.type}`, event)
  }

  async getEvents(
    taskId: string,
    limit?: number,
    offset = 0,
    eventType?: EventType,
  ): Promise<Event[]> {
    let events = await this.storage.getEvents(taskId)

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

    this.on('event', wrappedHandler)

    return () => {
      this.off('event', wrappedHandler)
    }
  }

  subscribeByType(eventType: EventType, handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch(console.error)
    }

    this.on(`type:${eventType}`, wrappedHandler)

    return () => {
      this.off(`type:${eventType}`, wrappedHandler)
    }
  }

  async getEventCount(taskId: string, eventType?: EventType): Promise<number> {
    const events = await this.getEvents(taskId, undefined, 0, eventType)
    return events.length
  }
}

export function createEventService(_app: FastifyInstance, storage: StorageBackend): EventService {
  return new EventService(storage)
}
