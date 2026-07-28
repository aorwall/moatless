/**
 * Live task events, read from the CLI's event stream.
 *
 * The stream is what the suite uses to observe work it did not initiate —
 * an agent finishing a turn, a sandbox becoming ready, a server starting.
 * Collection happens in the background; the query methods read what has
 * arrived so far, and the `waitFor*` methods poll that buffer.
 */

import type { Moat, StreamHandle } from './moat'

export interface StreamEvent {
  eventId: string
  type: string
  subType?: string
  taskId: string
  timestamp?: string
  isError: boolean
  payload: Record<string, unknown>
}

export interface TaskLifecycleVerification {
  success: boolean
  errors: string[]
  receivedEvents: string[]
}

/** The events a completed task turn is expected to have produced. */
const TASK_LIFECYCLE: Array<{
  type: string
  subType?: string
  description: string
}> = [
  { type: 'task', description: 'Task event' },
  { type: 'environment', subType: 'ready', description: 'Environment ready' },
  { type: 'server', subType: 'started', description: 'Server started' },
  { type: 'taskTurn', subType: 'active', description: 'Iteration active' },
  { type: 'agent', subType: 'started', description: 'Agent started' },
  { type: 'agent', subType: 'completed', description: 'Agent completed' },
  { type: 'taskTurn', subType: 'completed', description: 'Iteration completed' },
]

const POLL_INTERVAL_MS = 500

export class EventStream {
  private events: StreamEvent[] = []
  private handle: StreamHandle | null = null
  private error: Error | null = null

  constructor(private readonly moat: Moat) {}

  /** Begin collecting. Optionally narrowed to one task. */
  async start(taskId?: string): Promise<void> {
    this.stop()
    this.events = []
    this.error = null

    const args = ['events', 'stream']
    if (taskId) {
      args.push('--task', taskId)
    }
    this.handle = this.moat.stream<StreamEvent>(
      args,
      (event) => {
        this.events.push(event)
      },
      (error) => {
        this.error = error
      },
    )

    // Give the connection a moment to establish, so an immediate failure
    // surfaces here rather than as a mysteriously empty buffer later.
    await sleep(1000)
    if (this.error) {
      throw this.error
    }
  }

  stop(): void {
    this.handle?.stop()
    this.handle = null
  }

  getError(): Error | null {
    return this.error
  }

  getAllEvents(): StreamEvent[] {
    return [...this.events]
  }

  getEventsForTask(taskId: string): StreamEvent[] {
    return this.events.filter((event) => event.taskId === taskId)
  }

  countEvents(type: string, subType: string, taskId: string): number {
    return this.events.filter(
      (event) => event.type === type && event.subType === subType && event.taskId === taskId,
    ).length
  }

  /** Wait for one matching event. Returns null on timeout. */
  async waitForEvent(options: {
    type: string
    subType?: string
    taskId?: string
    timeout?: number
  }): Promise<StreamEvent | null> {
    const { type, subType, taskId, timeout = 60_000 } = options
    return this.poll(timeout, () =>
      findLast(
        this.events,
        (event) =>
          event.type === type &&
          (subType === undefined || event.subType === subType) &&
          (taskId === undefined || event.taskId === taskId),
      ),
    )
  }

  /** Wait for whichever of several events arrives first. */
  async waitForAnyEvent(options: {
    events: Array<{ type: string; subType?: string }>
    taskId?: string
    timeout?: number
  }): Promise<StreamEvent | null> {
    const { events: patterns, taskId, timeout = 60_000 } = options
    return this.poll(timeout, () =>
      findLast(
        this.events,
        (event) =>
          (taskId === undefined || event.taskId === taskId) &&
          patterns.some(
            (pattern) =>
              event.type === pattern.type &&
              (pattern.subType === undefined || event.subType === pattern.subType),
          ),
      ),
    )
  }

  /** Wait until the Nth matching event has arrived, and return it. */
  async waitForNthEvent(
    type: string,
    subType: string,
    taskId: string,
    n: number,
    timeoutMs = 300_000,
  ): Promise<StreamEvent | null> {
    return this.poll(timeoutMs, () => {
      const matching = this.events.filter(
        (event) => event.type === type && event.subType === subType && event.taskId === taskId,
      )
      return matching.length >= n ? matching[n - 1] : undefined
    })
  }

  /** Which of the expected lifecycle events a task produced, and which it did not. */
  verifyTaskLifecycle(taskId: string): TaskLifecycleVerification {
    const taskEvents = this.getEventsForTask(taskId)
    const errors: string[] = []
    const receivedEvents: string[] = []

    for (const required of TASK_LIFECYCLE) {
      const key = required.subType ? `${required.type}.${required.subType}` : required.type
      const found = taskEvents.some(
        (event) =>
          event.type === required.type &&
          (required.subType === undefined || event.subType === required.subType),
      )
      found ? receivedEvents.push(key) : errors.push(`Missing: ${required.description} (${key})`)
    }

    const messages = taskEvents.filter(
      (event) => event.type === 'agent' && event.subType === 'message',
    )
    messages.length > 0
      ? receivedEvents.push(`agent.message (${messages.length})`)
      : errors.push('Missing: Agent messages (agent.message)')

    return { success: errors.length === 0, errors, receivedEvents }
  }

  /** Counts by `type.subType`, for a failure message worth reading. */
  getEventSummary(taskId?: string): Record<string, number> {
    const events = taskId ? this.getEventsForTask(taskId) : this.events
    const summary: Record<string, number> = {}
    for (const event of events) {
      const key = event.subType ? `${event.type}.${event.subType}` : event.type
      summary[key] = (summary[key] || 0) + 1
    }
    return summary
  }

  private async poll<T>(timeoutMs: number, attempt: () => T | undefined): Promise<T | null> {
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline) {
      const found = attempt()
      if (found !== undefined) {
        return found
      }
      await sleep(POLL_INTERVAL_MS)
    }
    return null
  }
}

function findLast<T>(items: T[], predicate: (item: T) => boolean): T | undefined {
  for (let i = items.length - 1; i >= 0; i--) {
    if (predicate(items[i])) {
      return items[i]
    }
  }
  return
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
