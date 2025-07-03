import type { MiddlewareHandler } from 'hono'
import { createLogger } from '../utils/logger.js'
import type { AppBindings } from '../app.js'

const appLogger = createLogger()

export const logger: MiddlewareHandler<AppBindings> = async (c, next) => {
  const start = Date.now()
  const method = c.req.method
  const path = c.req.path
  const requestId = c.get('requestId')

  await next()

  const status = c.res.status
  const duration = Date.now() - start

  // Skip logging for health checks in production
  if (process.env.NODE_ENV === 'production' && path === '/health') {
    return
  }

  appLogger.info(`${method} ${path} ${status} ${duration}ms`, {
    method,
    path,
    status,
    duration,
    requestId,
  })
}