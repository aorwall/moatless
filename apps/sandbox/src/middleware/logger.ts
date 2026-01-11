import type { Context, MiddlewareHandler, Next } from 'hono'

/**
 * Request logging middleware
 * Uses stdout directly for request logging (structured logging handled by pino elsewhere)
 */
export const loggerMiddleware: MiddlewareHandler = async (c: Context, next: Next) => {
  const start = Date.now()
  const { method, path } = c.req

  await next()

  const duration = Date.now() - start
  const status = c.res.status

  // Log request using process.stdout for simple request logging
  // biome-ignore lint/suspicious/noConsole: Request logging to stdout is intentional
  console.log(`${method} ${path} ${status} ${duration}ms`)
}
