import type { MiddlewareHandler } from 'hono'
import type { AppBindings } from '../app'
import { logger as appLogger } from '../utils/logger'

export const logger: MiddlewareHandler<AppBindings> = async (c, next) => {
  const start = Date.now()
  const method = c.req.method
  const path = c.req.path
  const requestId = c.get('requestId')

  await next()

  const status = c.res.status
  const duration = Date.now() - start

  // Skip logging for health checks and metrics in production (too noisy)
  if (process.env['NODE_ENV'] === 'production' && (path === '/health' || path === '/metrics')) {
    return
  }

  if (status >= 500) {
    appLogger.info(
      {
        method,
        path,
        status,
        duration,
        requestId,
      },
      `${method} ${path} ${status} ${duration}ms`,
    )
  } else if (status >= 400) {
    appLogger.warn(
      {
        method,
        path,
        status,
        duration,
        requestId,
      },
      `${method} ${path} ${status} ${duration}ms`,
    )
  } else if ((method === 'GET' || method === 'OPTIONS') && status >= 200 && status < 300) {
    // Log GET and OPTIONS requests with 2xx status at DEBUG level
    appLogger.debug(
      {
        method,
        path,
        status,
        duration,
        requestId,
      },
      `${method} ${path} ${status} ${duration}ms`,
    )
  } else {
    appLogger.info(
      {
        method,
        path,
        status,
        duration,
        requestId,
      },
      `${method} ${path} ${status} ${duration}ms`,
    )
  }
}
