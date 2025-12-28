import type { Event } from '@moatless/types'
import { logger } from '../../../utils/logger'
import { type AgentOptions, type AgentRunOptions, BaseAgent } from '../../agents/agent/base-agent'
import type { EventService } from '../../events/services/event.service'

/**
 * Configuration for RemoteAgentProxy
 */
export interface RemoteAgentProxyConfig {
  podIp: string
  workerPort: number
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

interface AgentStatusResponse {
  state: 'idle' | 'running' | 'completed'
}

/**
 * RemoteAgentProxy - Proxies agent execution to a Kubernetes pod
 *
 * This agent proxy:
 * 1. Calls the pod's /execute-agent endpoint to start the agent
 * 2. Polls the pod for agent status and events
 * 3. Calls the pod's /stop-agent endpoint when stop() is called
 *
 * Events are forwarded from the pod via the WorkerApiClient which calls
 * the main backend's POST /api/v1/worker/emit-event endpoint. The EventService
 * then broadcasts these events to subscribers.
 */
export class RemoteAgentProxy extends BaseAgent {
  private proxyAgentType: string
  private abortController: AbortController | null = null
  private workerBaseUrl: string
  private eventService: EventService

  constructor(config: RemoteAgentProxyConfig, options: AgentOptions = {}) {
    super(config.taskId, options)
    this.proxyAgentType = config.agentType
    this.workerBaseUrl = `http://${config.podIp}:${config.workerPort}`
    this.eventService = config.eventService
  }

  getAgentType(): string {
    return this.proxyAgentType
  }

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex agent lifecycle with polling, event streaming, and error handling
  async *run(message: string, options?: AgentRunOptions): AsyncGenerator<Event> {
    this.stopRequested = false
    this.abortController = new AbortController()

    // Event queue for bridging EventEmitter to async generator
    const eventQueue: Event[] = []
    let resolveWait: (() => void) | null = null
    let agentCompleted = false

    // Subscribe to events for this task BEFORE starting agent
    // Events from worker pod flow: Worker → POST /emit-event → EventService → here
    const unsubscribe = this.eventService.subscribe(this.taskId, (event) => {
      eventQueue.push(event)
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
        'RemoteAgentProxy: Agent started on pod, streaming events',
      )

      // Yield events as they arrive while polling for completion
      while (!(this.stopRequested || agentCompleted)) {
        // Yield all queued events
        while (eventQueue.length > 0) {
          const event = eventQueue.shift()
          if (event) {
            yield event
          }
        }

        // Check if agent completed on pod
        try {
          const statusResponse = await fetch(`${this.workerBaseUrl}/agent-status`, {
            signal: this.abortController.signal,
          })

          if (statusResponse.ok) {
            const status = (await statusResponse.json()) as AgentStatusResponse
            if (status.state === 'idle' || status.state === 'completed') {
              logger.info(
                { taskId: this.taskId, state: status.state },
                'RemoteAgentProxy: Agent completed on pod',
              )
              agentCompleted = true
              break
            }
          }
        } catch {
          // Status check failed - agent might still be running
          if (this.stopRequested) {
            break
          }
        }

        // Wait for next event or timeout (500ms for responsive status checks)
        await Promise.race([
          new Promise<void>((resolve) => {
            resolveWait = resolve
          }),
          new Promise<void>((resolve) => setTimeout(resolve, 500)),
        ])
      }

      // Yield any remaining events after agent completed
      while (eventQueue.length > 0) {
        const event = eventQueue.shift()
        if (event) {
          yield event
        }
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
