/**
 * @moatless/messaging
 *
 * Unified messaging package for events and request queues.
 * Provides abstractions and implementations for cross-process communication.
 */

// Events module
export {
  BaseEventService,
  type EventHandler,
  type EventLogger,
  type EventQueryOptions,
  type EventRepository,
  type EventServiceOptions,
  type EventStorage,
  type EventTransport,
} from './events/index.js'
// Queue module
export type {
  PendingMessage,
  QueueMessage,
  ReceiveOptions,
  RequestQueue,
} from './queue/index.js'
// Redis transports
export {
  type RedisClientOptions,
  RedisEventTransport,
  type RedisEventTransportOptions,
  RedisMessagingClient,
  RedisRequestQueue,
  type RedisRequestQueueOptions,
} from './transports/redis/index.js'
// Common types
export { type MessagingClient, type MessagingLogger, noopLogger } from './types.js'
