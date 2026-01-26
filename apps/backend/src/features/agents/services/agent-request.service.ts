import type { PendingMessage, QueueMessage, RequestQueue } from '@moatless/messaging'
import type { AgentRunRequest } from '@moatless/sandbox-schemas'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { RequestQueueSymbol } from '../symbols'

/**
 * Stream key prefix for agent requests
 */
const AGENT_REQUEST_STREAM_PREFIX = 'agent-requests'

/**
 * AgentRequestService - Publishes agent execution requests to queue
 *
 * This service replaces the direct HTTP fire-and-forget pattern with
 * a Redis Streams-based queue. The sandbox consumes requests from
 * the queue and executes agents.
 *
 * Benefits:
 * - Reliable delivery (at-least-once via consumer groups)
 * - Recovery on sandbox restart (unACKed messages redeliver)
 * - Unified execution flow for createTask and sendMessage
 */
@injectable()
export class AgentRequestService {
  constructor(
    @inject(RequestQueueSymbol)
    private readonly requestQueue: RequestQueue<AgentRunRequest>,
  ) {}

  /**
   * Get the stream key for a task's agent requests
   */
  private getStreamKey(taskId: string): string {
    return `${AGENT_REQUEST_STREAM_PREFIX}:${taskId}`
  }

  /**
   * Publish an agent execution request to the queue
   *
   * @param taskId - Task ID
   * @param request - Agent run request with content and options
   * @returns Message ID from the queue
   */
  async publish(taskId: string, request: AgentRunRequest): Promise<string> {
    const streamKey = this.getStreamKey(taskId)

    logger.info(
      {
        taskId,
        streamKey,
        model: request.model,
        agentMode: request.agentMode,
        hasResume: !!request.resume,
        contentLength: request.content.length,
      },
      'AgentRequestService: Publishing agent request to queue',
    )

    try {
      const messageId = await this.requestQueue.publish(streamKey, request)

      logger.debug(
        {
          taskId,
          messageId,
        },
        'AgentRequestService: Request published successfully',
      )

      return messageId
    } catch (error) {
      logger.error(
        {
          err: error as Error,
          taskId,
        },
        'AgentRequestService: Failed to publish request',
      )
      throw error
    }
  }

  /**
   * List pending (unacknowledged) requests for a task
   *
   * @param taskId - Task ID
   * @returns List of pending messages
   */
  async listPending(taskId: string): Promise<PendingMessage[]> {
    const streamKey = this.getStreamKey(taskId)
    return await this.requestQueue.listPending(streamKey)
  }

  /**
   * Get a request from the queue (for sandbox consumer)
   *
   * @param taskId - Task ID
   * @param blockMs - Time to block waiting for message
   * @returns Queue message or null
   */
  async receive(taskId: string, blockMs?: number): Promise<QueueMessage<AgentRunRequest> | null> {
    const streamKey = this.getStreamKey(taskId)
    return await this.requestQueue.receive(
      streamKey,
      blockMs !== undefined ? { blockMs } : undefined,
    )
  }

  /**
   * Acknowledge a request as processed
   *
   * @param taskId - Task ID
   * @param messageId - Message ID to acknowledge
   */
  async acknowledge(taskId: string, messageId: string): Promise<void> {
    const streamKey = this.getStreamKey(taskId)
    await this.requestQueue.acknowledge(streamKey, messageId)
  }
}
