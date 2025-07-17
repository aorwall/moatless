import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { ClaudeCodeSettings, Event, EventType, RepositoryConfig, Task } from '@moatless/types'
import { getClaudeService } from '../agents/claude-code/claude.service.js'
import { sdkMessageToEvent } from '../utils/claude-code-events.js'
import type { StorageBackend } from './storage.js'

export type EventHandler = (event: Event) => void | Promise<void>

export class EventService extends EventEmitter {
  private eventCache: Map<string, Event[]> = new Map()
  private repository?: RepositoryConfig

  constructor(
    private storage: StorageBackend,
    _claudeCodeSettings?: ClaudeCodeSettings,
    repository?: RepositoryConfig,
  ) {
    super()
    this.setMaxListeners(100)
    // claudeCodeSettings is no longer used directly since we use Claude service
    this.repository = repository
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
      // If task is provided and agent type is claude-code, read from Claude events disk (default behavior)
      if (task && task.agentType === 'claude-code' && !process.env.SKIP_CLAUDE_DISK_EVENTS) {
        try {
          // Use Claude service to read messages
          const claudeService = getClaudeService()
          const sdkMessages = await claudeService.readClaudeMessages(taskId, task, this.repository)

          // Convert SDK messages to events
          events = []
          for (const sdkMessage of sdkMessages) {
            // For Claude service, we already have messages grouped by iteration
            // Find the appropriate iteration number based on timestamp or other criteria
            let iterationNumber: number | undefined

            // Try to match by timestamp - find the iteration this message belongs to
            const messageTimestamp =
              'timestamp' in sdkMessage ? new Date(sdkMessage.timestamp as any) : new Date()

            for (const iteration of task.iterations) {
              if (
                iteration.startTime <= messageTimestamp &&
                (!iteration.endTime || iteration.endTime >= messageTimestamp)
              ) {
                iterationNumber = iteration.iterationNumber
                break
              }
            }

            const event = sdkMessageToEvent(sdkMessage, taskId, iterationNumber)
            events.push(event)
          }
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

export function createEventService(
  storage: StorageBackend,
  claudeCodeSettings?: ClaudeCodeSettings,
  repository?: RepositoryConfig,
): EventService {
  return new EventService(storage, claudeCodeSettings, repository)
}
