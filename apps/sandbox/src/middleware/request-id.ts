import { randomUUID } from 'node:crypto'
import type { Context, MiddlewareHandler, Next } from 'hono'

/**
 * Request ID middleware - adds unique ID to each request
 */
export const requestIdMiddleware: MiddlewareHandler = async (c: Context, next: Next) => {
  const requestId = c.req.header('x-request-id') || randomUUID()
  c.set('requestId', requestId)
  c.header('x-request-id', requestId)
  await next()
}
