import type { Context, Next } from 'hono'
import { SessionContext } from '../../../features/auth/index'

/**
 * Session context middleware
 * Stores the session in AsyncLocalStorage for the request lifecycle
 *
 * This middleware should be placed after authentication middleware
 * but before route handlers to ensure session is available throughout request
 */
export const sessionContext = (c: Context, next: Next) => {
  // Get session from context (set by auth middleware if authenticated)
  const session = c.get('session')

  // Run the rest of the request within the session context
  // This makes the session available via SessionContext.getCurrentSession()
  // throughout the request lifecycle without passing it explicitly
  return SessionContext.run(session, async () => {
    await next()
  })
}
