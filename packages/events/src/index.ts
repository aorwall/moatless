/**
 * @moatless/events
 *
 * Re-exports from @moatless/messaging for backwards compatibility.
 * New code should import directly from @moatless/messaging.
 */

export {
  BaseEventService,
  type EventHandler,
  type EventLogger,
  type EventQueryOptions,
  type EventRepository,
  type EventServiceOptions,
  type EventStorage,
  type EventTransport,
} from '@moatless/messaging'
