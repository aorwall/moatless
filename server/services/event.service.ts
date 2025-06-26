import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { ClaudeCodeSettings } from '@shared/types/claude.js'
import type { FastifyInstance } from 'fastify'
import type { Event, EventType } from '../../shared/types/index.js'
import type { Task } from '../../shared/types/task.js'
import { getDefaultClaudeCodeSettings, readClaudeCodeEvents } from '../utils/claude-code-events.js'
import type { StorageBackend } from './storage.js'

export type EventHandler = (event: Event) => void | Promise<void>

export class EventService extends EventEmitter {
  private claudeCodeSettings: ClaudeCodeSettings
  private eventCache: Map<string, Event[]> = new Map()

  constructor(
    private storage: StorageBackend,
    claudeCodeSettings?: ClaudeCodeSettings,
  ) {
    super()
    this.setMaxListeners(100)
    this.claudeCodeSettings = claudeCodeSettings || getDefaultClaudeCodeSettings()
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
      // If task is provided and agent type is claude-code, read from Claude events disk (default behavior)
      if (task && task.agentType === 'claude-code' && !process.env.SKIP_CLAUDE_DISK_EVENTS) {
        try {
          events = await readClaudeCodeEvents(this.claudeCodeSettings, task)
        } catch (error) {
          // Log error but don't fail the request
          console.error(`Failed to read Claude Code events from disk for task ${taskId}:`, error)
          throw error
        }
      } else {
        // Otherwise read from our storage
        events = await this.storage.getEvents(taskId)
      }
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

    this.on('event', wrappedHandler)

    return () => {
      this.off('event', wrappedHandler)
    }
  }

  async getEventCount(taskId: string, eventType?: EventType, task?: Task): Promise<number> {
    const events = await this.getEvents(taskId, undefined, 0, eventType, task)
    return events.length
  }

  clearCache(taskId?: string): void {
    if (taskId) {
      this.eventCache.delete(taskId)
    } else {
      this.eventCache.clear()
    }
  }
}

export function createEventService(
  _app: FastifyInstance,
  storage: StorageBackend,
  claudeCodeSettings?: ClaudeCodeSettings,
): EventService {
  return new EventService(storage, claudeCodeSettings)
}
