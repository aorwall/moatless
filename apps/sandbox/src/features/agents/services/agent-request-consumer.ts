import type { QueueMessage, RedisRequestQueue } from '@moatless/messaging'
import type { AgentRunRequest } from '@moatless/sandbox-schemas'
import type { AskUserQuestionResponse, ExitPlanModeResponse } from '@moatless/types'
import type { SandboxConfig } from '../../../config/environment'
import { createBackendApiClient } from '../../../shared/backend-api-client'
import { logger } from '../../../shared/logger'
import type { SandboxEventService } from '../../events/services/sandbox-event.service'
import { createSandboxTools } from '../../tools'
import {
  getActivePromptToolName,
  hasActivePrompt,
  resolveInteractivePrompt,
} from '../claude-code/interactive-prompt'
import { AgentService } from './agent.service'

/**
 * Stream key prefix for agent requests
 */
const AGENT_REQUEST_STREAM_PREFIX = 'agent-requests'

/**
 * Maximum number of messages to batch together
 */
const MAX_BATCH_SIZE = 10

/**
 * Timeout for accumulating additional messages (ms)
 * Short timeout to minimize latency while allowing batching
 */
const BATCH_ACCUMULATION_TIMEOUT_MS = 100

/**
 * Convert a free-form message to the appropriate user input response format.
 * For AskUserQuestion: Uses the message as a freeform answer
 * For ExitPlanMode: Any message is treated as rejection with the message as feedback
 */
function convertMessageToUserInputResponse(
  toolName: 'AskUserQuestion' | 'ExitPlanMode' | null,
  messageText: string,
): AskUserQuestionResponse | ExitPlanModeResponse {
  if (toolName === 'ExitPlanMode') {
    // Any message while waiting for plan approval is treated as rejection
    return {
      approved: false,
      feedback: messageText,
    }
  }

  // Default: AskUserQuestion - use message as freeform response
  return {
    answers: { _freeformResponse: messageText },
  }
}

/**
 * AgentRequestConsumer - Consumes agent execution requests from Redis queue
 *
 * This service replaces the HTTP POST endpoint (/execute-agent) with a
 * Redis Streams consumer. It polls for new requests and executes agents.
 *
 * Features:
 * - Reliable delivery (at-least-once via consumer groups)
 * - Recovery on sandbox restart (unACKed messages redeliver)
 * - Unified execution flow for createTask and sendMessage
 * - Request batching: Multiple queued requests are batched into a single SDK call
 */
export class AgentRequestConsumer {
  private running = false
  private agentService: AgentService

  constructor(
    private readonly requestQueue: RedisRequestQueue<AgentRunRequest>,
    private readonly config: SandboxConfig,
    eventService: SandboxEventService | undefined,
  ) {
    const backendApi = createBackendApiClient(config)
    const tools = createSandboxTools(config, backendApi)
    this.agentService = new AgentService(config.taskId, config.workspaceDir, eventService, tools)
  }

  /**
   * Get the stream key for this sandbox's agent requests
   */
  private getStreamKey(): string {
    return `${AGENT_REQUEST_STREAM_PREFIX}:${this.config.taskId}`
  }

  /**
   * Start the consumer loop
   */
  // biome-ignore lint/suspicious/useAwait: consumeLoop runs in background intentionally
  async start(): Promise<void> {
    if (this.running) {
      logger.warn({ taskId: this.config.taskId }, 'AgentRequestConsumer: Already running')
      return
    }

    this.running = true

    logger.info(
      {
        taskId: this.config.taskId,
        streamKey: this.getStreamKey(),
      },
      'AgentRequestConsumer: Starting consumer loop',
    )

    // Run consumer loop in background
    this.consumeLoop().catch((error) => {
      logger.error(
        { err: error, taskId: this.config.taskId },
        'AgentRequestConsumer: Consumer loop crashed',
      )
    })
  }

  /**
   * Stop the consumer loop
   */
  stop(): void {
    this.running = false
    logger.info({ taskId: this.config.taskId }, 'AgentRequestConsumer: Stopping consumer')
  }

  /**
   * Main consume loop - polls Redis for requests with batching support
   */
  private async consumeLoop(): Promise<void> {
    const streamKey = this.getStreamKey()

    while (this.running) {
      try {
        // Blocking read with 5 second timeout for first message
        const firstMessage = await this.requestQueue.receive(streamKey, { blockMs: 5000 })

        if (!firstMessage) {
          // No message, continue polling
          continue
        }

        logger.info(
          {
            taskId: this.config.taskId,
            messageId: firstMessage.messageId,
            model: firstMessage.payload.model,
            agentMode: firstMessage.payload.agentMode,
            hasResume: !!firstMessage.payload.resume,
          },
          'AgentRequestConsumer: Received first request',
        )

        // Accumulate additional messages with short non-blocking reads
        const messages = await this.accumulateMessages(streamKey, firstMessage)

        const batchSize = messages.length

        if (batchSize > 1) {
          logger.info(
            {
              taskId: this.config.taskId,
              batchSize,
              messageIds: messages.map((m) => m.messageId),
            },
            'AgentRequestConsumer: Batched multiple requests',
          )
        }

        try {
          // Execute the batch
          await this.executeBatch(messages.map((m) => m.payload))

          // ACK all messages on success
          for (const message of messages) {
            await this.requestQueue.acknowledge(streamKey, message.messageId)
          }

          logger.info(
            {
              taskId: this.config.taskId,
              batchSize,
              messageIds: messages.map((m) => m.messageId),
            },
            'AgentRequestConsumer: Batch completed and acknowledged',
          )
        } catch (error) {
          // ACK all messages on failure too (don't retry indefinitely)
          // Error is already emitted as event by AgentService
          for (const message of messages) {
            await this.requestQueue.acknowledge(streamKey, message.messageId)
          }

          logger.error(
            {
              err: error,
              taskId: this.config.taskId,
              batchSize,
              messageIds: messages.map((m) => m.messageId),
            },
            'AgentRequestConsumer: Batch failed, acknowledged to prevent redelivery',
          )
        }
      } catch (error) {
        if (!this.running) {
          // Expected when stopping
          break
        }

        logger.error(
          { err: error, taskId: this.config.taskId },
          'AgentRequestConsumer: Error in consume loop, retrying',
        )

        // Backoff before retrying
        await this.sleep(2000)
      }
    }

    logger.info({ taskId: this.config.taskId }, 'AgentRequestConsumer: Consumer loop exited')
  }

  /**
   * Accumulate additional messages from the queue with a short timeout
   *
   * After receiving the first message, perform non-blocking reads to
   * accumulate any additional queued messages up to MAX_BATCH_SIZE.
   */
  private async accumulateMessages(
    streamKey: string,
    firstMessage: QueueMessage<AgentRunRequest>,
  ): Promise<QueueMessage<AgentRunRequest>[]> {
    const messages: QueueMessage<AgentRunRequest>[] = [firstMessage]

    // Try to accumulate more messages with short non-blocking reads
    while (messages.length < MAX_BATCH_SIZE) {
      const additionalMessage = await this.requestQueue.receive(streamKey, {
        blockMs: BATCH_ACCUMULATION_TIMEOUT_MS,
      })

      if (!additionalMessage) {
        // No more messages available, stop accumulating
        break
      }

      logger.debug(
        {
          taskId: this.config.taskId,
          messageId: additionalMessage.messageId,
          currentBatchSize: messages.length + 1,
        },
        'AgentRequestConsumer: Accumulated additional message',
      )

      messages.push(additionalMessage)
    }

    return messages
  }

  /**
   * Execute a batch of agent requests
   */
  private async executeBatch(requests: AgentRunRequest[]): Promise<void> {
    const state = this.agentService.getState()

    if (state.state === 'running') {
      // Check if agent is waiting for user input - if so, use message as response
      if (hasActivePrompt()) {
        const pendingTool = getActivePromptToolName()

        // Extract text from the first request's content
        const messageText = requests[0].content
          .filter((c): c is { type: 'text'; text: string } => c.type === 'text')
          .map((c) => c.text)
          .join('\n')

        logger.info(
          { taskId: this.config.taskId, pendingTool, messageLength: messageText.length },
          'AgentRequestConsumer: Agent waiting for input, using incoming message as response',
        )

        // Convert message to appropriate response format and resolve
        const response = convertMessageToUserInputResponse(pendingTool, messageText)
        const resolved = resolveInteractivePrompt(response)

        if (resolved) {
          logger.info(
            { taskId: this.config.taskId, pendingTool },
            'AgentRequestConsumer: Pending input resolved with message',
          )
          return // Successfully used message as response, don't throw
        }
        // If resolve failed, fall through to error
      }

      throw new Error('Agent is already running')
    }

    if (requests.length === 1) {
      // Single request, use simple execution
      const result = await this.agentService.executeRequest(requests[0])

      if (!result.success) {
        throw new Error(result.error || 'Agent execution failed')
      }
    } else {
      // Multiple requests, create and execute batched request
      const batchedRequest = this.agentService.createBatchedRequest(requests)
      const result = await this.agentService.executeBatchedRequest(batchedRequest)

      if (!result.success) {
        throw new Error(result.error || 'Batched agent execution failed')
      }
    }
  }

  /**
   * Get current agent state
   */
  getState(): { state: 'idle' | 'running'; hasAgent: boolean } {
    const serviceState = this.agentService.getState()
    return {
      state: serviceState.state,
      hasAgent: serviceState.hasAgent,
    }
  }

  /**
   * Stop current agent
   */
  async stopAgent(): Promise<void> {
    await this.agentService.stopAgent()
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}

/**
 * Create and initialize the agent request consumer
 */
export async function createAgentRequestConsumer(
  config: SandboxConfig,
  eventService: SandboxEventService | undefined,
): Promise<AgentRequestConsumer | undefined> {
  const redisUrl = process.env['REDIS_URL']

  if (!redisUrl) {
    logger.warn('REDIS_URL not set, agent request consumer disabled')
    return undefined
  }

  try {
    const { RedisRequestQueue } = await import('@moatless/messaging')

    const requestQueue = new RedisRequestQueue<AgentRunRequest>({
      redisUrl,
      logger: {
        debug: (obj, msg) => logger.debug(obj, msg),
        info: (obj, msg) => logger.info(obj, msg),
        warn: (obj, msg) => logger.warn(obj, msg),
        error: (obj, msg) => logger.error(obj, msg),
      },
      consumerGroup: `sandbox-${config.taskId}-consumers`,
      consumerId: `sandbox-${config.taskId}-worker`,
    })

    await requestQueue.initialize()

    const consumer = new AgentRequestConsumer(requestQueue, config, eventService)

    logger.info(
      {
        taskId: config.taskId,
        redisUrl,
      },
      'Agent request consumer initialized',
    )

    return consumer
  } catch (error) {
    logger.error({ err: error }, 'Failed to create agent request consumer')
    return undefined
  }
}
