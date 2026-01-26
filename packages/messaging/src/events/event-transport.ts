import type { Event } from '@moatless/types'
import type { MessagingClient } from '../types.js'

/**
 * Transport for cross-process event delivery
 */
export interface EventTransport extends MessagingClient {
  /**
   * Publish an event to the transport
   * @returns Promise that resolves when event is published (not delivered)
   */
  publishEvent(event: Event): Promise<void>

  /**
   * Start listening for events (for consumers)
   * @param handler - Callback invoked when event is received
   */
  startListening(handler: (event: Event) => Promise<void>): Promise<void>

  /**
   * Stop listening for events
   */
  stopListening(): Promise<void>
}
