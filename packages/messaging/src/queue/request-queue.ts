import type { MessagingClient } from '../types.js'
import type { PendingMessage, QueueMessage, ReceiveOptions } from './queue-types.js'

/**
 * Request queue interface for agent execution requests.
 *
 * Provides a generic queue abstraction that can be implemented with
 * different backends (Redis Streams, AWS SQS, etc.).
 *
 * Key concepts:
 * - Messages are keyed by a string (e.g., "agent-requests:{taskId}")
 * - Publishers add messages to the queue
 * - Consumers receive messages and must acknowledge them after processing
 * - Unacknowledged messages can be redelivered
 */
export interface RequestQueue<T = unknown> extends MessagingClient {
  /**
   * Publish a message to the queue
   *
   * @param key - Queue key (e.g., "agent-requests:{taskId}")
   * @param payload - The message payload
   * @returns Message ID
   */
  publish(key: string, payload: T): Promise<string>

  /**
   * Receive a message from the queue
   *
   * Uses blocking reads for efficient polling. If no message is available,
   * blocks up to blockMs before returning null.
   *
   * @param key - Queue key (e.g., "agent-requests:{taskId}")
   * @param options - Receive options (blockMs, count)
   * @returns Message or null if no message available
   */
  receive(key: string, options?: ReceiveOptions): Promise<QueueMessage<T> | null>

  /**
   * Acknowledge a message as processed
   *
   * Must be called after successfully processing a message.
   * Unacknowledged messages may be redelivered.
   *
   * @param key - Queue key
   * @param messageId - Message ID to acknowledge
   */
  acknowledge(key: string, messageId: string): Promise<void>

  /**
   * List pending (unacknowledged) messages
   *
   * Useful for monitoring and debugging.
   *
   * @param key - Queue key
   * @returns List of pending messages
   */
  listPending(key: string): Promise<PendingMessage[]>
}
