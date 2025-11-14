import type { UserSession } from '@moatless/types'
import type { Context, Next } from 'hono'

/**
 * Dev session middleware
 * Creates a mock user session when AUTH_PROVIDER is disabled
 *
 * This middleware should be placed before session-context middleware
 * to inject a dev user session into the context
 */
export const devSession = (c: Context, next: Next) => {
  const authProvider = process.env['AUTH_PROVIDER']

  // Only inject dev session when auth is disabled
  if (authProvider === 'disabled' || !authProvider) {
    const devUserId = process.env['DEV_USER_ID'] || 'dev-user'
    const devUserEmail = process.env['DEV_USER_EMAIL'] || 'dev@localhost'
    const devUserName = process.env['DEV_USER_NAME'] || 'Dev User'

    // Create a mock session with dev user
    const devUserSession: UserSession = {
      id: 'dev-session',
      provider: 'dev',
      userId: devUserId,
      userLogin: devUserId,
      userEmail: devUserEmail,
      userName: devUserName,
      avatarUrl: null,
      createdAt: new Date(),
      expiresAt: new Date(Date.now() + 365 * 24 * 60 * 60 * 1000), // 1 year
    }

    // Set the dev session in context so session-context middleware can use it
    c.set('session', devUserSession)
  }

  return next()
}
