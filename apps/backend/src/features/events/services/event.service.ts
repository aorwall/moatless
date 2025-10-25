import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { Event, EventType } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger.js'
import { EventsStorage } from '../storage/events.storage.js'

type EventHandler = (event: Event) => void | Promise<void>

@injectable()
export class EventService extends EventEmitter {
  private eventCache: Map<string, Event[]> = new Map()

  constructor(@inject(EventsStorage) private storage: EventsStorage) {
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
    const cachedEvents = this.eventCache.get(event.taskId)
    if (!cachedEvents) {
      throw new Error(`Event cache not found for task ${event.taskId}`)
    }
    cachedEvents.push(event)

    // Only save to disk if event is not a claude-code agent/task/taskIteration event
    const isClaudeCodeEvent =
      (event.type === 'agent' || event.type === 'task' || event.type === 'taskIteration') &&
      'agentType' in event &&
      event.agentType === 'claude-code'
    if (!isClaudeCodeEvent) {
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
  ): Promise<Event[]> {
    let events: Event[] = []

    // Check cache first
    const cachedEvents = this.eventCache.get(taskId)
    if (cachedEvents) {
      events = cachedEvents
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
      Promise.resolve(handler(event)).catch((error) => {
        logger.error('Event handler error:', error)
      })
    }

    this.on(`task:${taskId}`, wrappedHandler)

    return () => {
      this.off(`task:${taskId}`, wrappedHandler)
    }
  }

  subscribeAll(handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch((error) => {
        logger.error('Event handler error:', error)
      })
    }

    this.on('global', wrappedHandler)

    return () => {
      this.off('global', wrappedHandler)
    }
  }

  async getEventCount(taskId: string, eventType?: EventType): Promise<number> {
    const events = await this.getEvents(taskId, undefined, 0, eventType)
    return events.length
  }
}
