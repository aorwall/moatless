/**
 * Queue types for agent request queuing
 */

/**
 * Message from the queue
 */
export interface QueueMessage<T = unknown> {
  /**
   * Unique message ID from the transport (e.g., Redis stream entry ID)
   */
  messageId: string

  /**
   * The payload data
   */
  payload: T

  /**
   * Timestamp when the message was published
   */
  timestamp: Date
}

/**
 * Options for receiving messages
 */
export interface ReceiveOptions {
  /**
   * Block time in milliseconds (0 = no blocking)
   * Default: 5000 (5 seconds)
   */
  blockMs?: number

  /**
   * Maximum number of messages to receive
   * Default: 1
   */
  count?: number
}

/**
 * Pending message info (for listPending)
 */
export interface PendingMessage {
  /**
   * Message ID
   */
  messageId: string

  /**
   * Consumer that owns this message
   */
  consumer: string

  /**
   * Time since delivery in milliseconds
   */
  idleTimeMs: number

  /**
   * Number of delivery attempts
   */
  deliveryCount: number
}
