/**
 * SSE Event Collector for the backend NDJSON event stream.
 *
 * Connects to GET /api/v1/events/stream and collects events in the background.
 * Provides helpers to wait for specific events by type/subType/taskId.
 */

import { BACKEND_URL } from './env'
const POLL_INTERVAL = 500

export interface SSEEvent {
  eventId: string
  type: string
  subType?: string
  taskId: string
  timestamp?: string
  isError: boolean
  payload: Record<string, unknown>
  raw: Record<string, unknown>
}

export class SSEEventCollector {
  private events: SSEEvent[] = []
  private abortController: AbortController | null = null
  private debug: boolean
  error: Error | null = null

  constructor(debug = false) {
    this.debug = debug
  }

  /**
   * Start collecting events from the NDJSON stream.
   */
  start(): void {
    this.abortController = new AbortController()
    this.streamEvents().catch((err) => {
      // AbortError is expected when stop() is called
      if (err instanceof Error && err.name !== 'AbortError') {
        this.error = err
      }
    })
  }

  /**
   * Stop collecting events.
   */
  stop(): void {
    if (this.abortController) {
      this.abortController.abort()
      this.abortController = null
    }
  }

  /**
   * Get all events for a specific task.
   */
  getEventsForTask(taskId: string): SSEEvent[] {
    return this.events.filter((e) => e.taskId === taskId)
  }

  /**
   * Get all collected events.
   */
  getAllEvents(): SSEEvent[] {
    return [...this.events]
  }

  /**
   * Wait for a specific event to appear.
   */
  async waitForEvent(
    type: string,
    subType?: string,
    taskId?: string,
    timeoutMs = 60_000,
  ): Promise<SSEEvent | null> {
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline) {
      for (let i = this.events.length - 1; i >= 0; i--) {
        const event = this.events[i]
        if (event.type !== type) {
          continue
        }
        if (subType && event.subType !== subType) {
          continue
        }
        if (taskId && event.taskId !== taskId) {
          continue
        }
        return event
      }
      await sleep(POLL_INTERVAL)
    }
    return null
  }

  /**
   * Wait until we've seen N events of a specific type for a task.
   * Returns the Nth event (1-indexed), or null on timeout.
   */
  async waitForNthEvent(
    type: string,
    subType: string,
    taskId: string,
    n: number,
    timeoutMs = 300_000,
  ): Promise<SSEEvent | null> {
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline) {
      const matching = this.events.filter(
        (e) => e.type === type && e.subType === subType && e.taskId === taskId,
      )
      if (matching.length >= n) {
        return matching[n - 1]
      }
      await sleep(POLL_INTERVAL)
    }
    return null
  }

  /**
   * Count events matching a type/subType for a task.
   */
  countEvents(type: string, subType: string, taskId: string): number {
    return this.events.filter(
      (e) => e.type === type && e.subType === subType && e.taskId === taskId,
    ).length
  }

  private async streamEvents(): Promise<void> {
    const resp = await fetch(`${BACKEND_URL}/api/v1/events/stream`, {
      headers: { Accept: 'text/event-stream' },
      signal: this.abortController!.signal,
    })

    if (!resp.ok) {
      throw new Error(`SSE stream failed: ${resp.status}`)
    }

    const reader = resp.body!.getReader()
    const decoder = new TextDecoder()
    let buffer = ''

    while (true) {
      const { done, value } = await reader.read()
      if (done) {
        break
      }

      buffer += decoder.decode(value, { stream: true })

      // Process complete lines
      const lines = buffer.split('\n')
      // Keep the last incomplete line in the buffer
      buffer = lines.pop() ?? ''

      for (const line of lines) {
        const trimmed = line.trim()
        if (!trimmed) {
          continue
        }

        try {
          const data = JSON.parse(trimmed) as Record<string, unknown>
          if (data.type === 'ping') {
            continue
          }

          const event: SSEEvent = {
            eventId: (data.eventId as string) ?? '',
            type: (data.type as string) ?? '',
            subType: data.subType as string | undefined,
            taskId: (data.taskId as string) ?? '',
            timestamp: data.timestamp as string | undefined,
            isError: (data.isError as boolean) ?? false,
            payload: (data.payload as Record<string, unknown>) ?? {},
            raw: data,
          }

          if (this.debug && event.taskId) {
            console.log(`  [SSE] ${event.type}.${event.subType ?? '*'} for ${event.taskId.slice(0, 8)}`)
          }

          this.events.push(event)
        } catch {
          // Skip malformed lines
        }
      }
    }
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
