import type { MiddlewareHandler } from 'hono'
import type { AppBindings } from '../app'
import { logger } from '../shared/logger'

/**
 * Request logging middleware using structured Pino logging.
 * Skips /health requests entirely to reduce noise.
 */
export const loggerMiddleware: MiddlewareHandler<AppBindings> = async (c, next) => {
  const { method, path } = c.req

  if (path === '/health') {
    await next()
    return
  }

  const start = Date.now()
  const requestId = c.get('requestId')

  await next()

  const status = c.res.status
  const duration = Date.now() - start
  const fields = { method, path, status, duration, requestId }
  const msg = `${method} ${path} ${status} ${duration}ms`

  if (status >= 500) {
    logger.error(fields, msg)
  } else if (status >= 400) {
    logger.warn(fields, msg)
  } else if ((method === 'GET' || method === 'OPTIONS') && status >= 200 && status < 300) {
    logger.debug(fields, msg)
  } else {
    logger.info(fields, msg)
  }
}
