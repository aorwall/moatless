/**
 * Re-export from @moatless/messaging for backwards compatibility.
 * New code should import directly from @moatless/messaging.
 */

export {
  RedisEventTransport,
  type RedisEventTransportOptions,
} from '@moatless/messaging'

/**
 * @deprecated Use RedisEventTransportOptions instead
 */
export type RedisTransportOptions = import('@moatless/messaging').RedisEventTransportOptions
