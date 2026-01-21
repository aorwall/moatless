import type { Event } from '@moatless/types'
import { logger } from '../../../utils/logger'
import { type AgentOptions, type AgentRunOptions, BaseAgent } from '../../agents/agent/base-agent'
import type { EventService } from '../../events/services/event.service'

/**
 * Configuration for RemoteAgentProxy
 */
export interface RemoteAgentProxyConfig {
  workerBaseUrl: string
  taskId: string
  agentType: string
  eventService: EventService
}

/**
 * Response types for pod API calls
 */
interface ExecuteAgentResponse {
  success: boolean
  error?: string
}

/**
 * RemoteAgentProxy - Proxies agent execution to a Kubernetes pod
 *
 * This agent proxy:
 * 1. Calls the pod's /execute-agent endpoint to start the agent
 * 2. Monitors for agent completion via EventService subscription (SSE events)
 * 3. Calls the pod's /stop-agent endpoint when stop() is called
 *
 * IMPORTANT: This proxy does NOT yield events. Events flow directly from
 * the worker pod to EventService (via POST /emit-event) and are handled by:
 * - SSE clients (real-time UI updates via events.routes.ts)
 * - AgentController (task state updates)
 *
 * Yielding events here would cause an infinite loop since task-execution-coordinator
 * re-emits events it receives from the agent generator back to EventService.
 */
export class RemoteAgentProxy extends BaseAgent {
  private proxyAgentType: string
  private abortController: AbortController | null = null
  private workerBaseUrl: string
  private eventService: EventService

  constructor(config: RemoteAgentProxyConfig, options: AgentOptions = {}) {
    super(config.taskId, options)
    this.proxyAgentType = config.agentType
    this.workerBaseUrl = config.workerBaseUrl
    this.eventService = config.eventService
  }

  getAgentType(): string {
    return this.proxyAgentType
  }

  async *run(message: string, options?: AgentRunOptions): AsyncGenerator<Event> {
    this.stopRequested = false
    this.abortController = new AbortController()

    let resolveWait: (() => void) | null = null
    let agentCompleted = false
    let completionEvent: Event | null = null

    // Subscribe to events for this task to detect completion early
    // Events from worker pod flow: Worker → PostgreSQL → EventService → here
    // We capture the completion event to yield it at the end for AgentController to process
    const unsubscribe = this.eventService.subscribe(this.taskId, (event) => {
      // Check for agent completion event to break out of polling earlier
      if (event.type === 'agent' && event.subType === 'completed') {
        agentCompleted = true
        completionEvent = event // Store for yielding
      }
      // Wake up the polling loop if it's waiting
      if (resolveWait) {
        resolveWait()
        resolveWait = null
      }
    })

    logger.info(
      {
        taskId: this.taskId,
        agentType: this.proxyAgentType,
        workerUrl: this.workerBaseUrl,
      },
      'RemoteAgentProxy: Starting agent execution on pod',
    )

    try {
      // Start agent on pod
      const response = await fetch(`${this.workerBaseUrl}/execute-agent`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          taskId: this.taskId,
          message,
          agentType: this.proxyAgentType,
          runOptions: options,
          model: options?.model || this.options.model,
        }),
        signal: this.abortController.signal,
      })

      if (!response.ok) {
        const errorData = (await response.json().catch(() => ({}))) as { error?: string }
        throw new Error(
          `Failed to start agent on pod: ${response.status} ${errorData.error || response.statusText}`,
        )
      }

      const result = (await response.json()) as ExecuteAgentResponse
      if (!result.success) {
        throw new Error(`Pod returned failure: ${result.error}`)
      }

      logger.info(
        { taskId: this.taskId },
        'RemoteAgentProxy: Agent started on pod, waiting for completion',
      )

      // Wait for completion event from EventService subscription
      // Events are the source of truth - no polling fallback needed
      // The EventService subscription calls resolveWait() when events arrive
      while (!(this.stopRequested || agentCompleted)) {
        await new Promise<void>((resolve) => {
          resolveWait = resolve
        })
      }
      // Yield completion event for AgentController to process
      // This enables status updates, and other post-processing
      if (completionEvent) {
        logger.info(
          { taskId: this.taskId },
          'RemoteAgentProxy: Yielding completion event for processing',
        )
        yield completionEvent
      }
    } catch (error) {
      if (this.stopRequested) {
        logger.info({ taskId: this.taskId }, 'RemoteAgentProxy: Agent stopped by request')
      } else {
        logger.error(
          { err: error as Error, taskId: this.taskId },
          'RemoteAgentProxy: Agent execution failed',
        )
        throw error
      }
    } finally {
      // Always unsubscribe from events
      unsubscribe()
      this.abortController = null
    }
  }

  async stop(): Promise<void> {
    this.stopRequested = true

    if (this.abortController) {
      this.abortController.abort()
    }

    logger.info({ taskId: this.taskId }, 'RemoteAgentProxy: Stopping agent on pod')

    try {
      const response = await fetch(`${this.workerBaseUrl}/stop-agent`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ taskId: this.taskId }),
        signal: AbortSignal.timeout(5000),
      })

      if (!response.ok) {
        logger.warn(
          { taskId: this.taskId, status: response.status },
          'RemoteAgentProxy: Stop agent request failed',
        )
      } else {
        logger.info({ taskId: this.taskId }, 'RemoteAgentProxy: Agent stop request sent')
      }
    } catch (error) {
      logger.warn(
        { err: error as Error, taskId: this.taskId },
        'RemoteAgentProxy: Failed to send stop request to pod',
      )
    }
  }
}
