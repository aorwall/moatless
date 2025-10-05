import type { UserSession } from '@moatless/types'
import type { Context, Next } from 'hono'
import { getCookie } from 'hono/cookie'
import { HTTPException } from 'hono/http-exception'
import { logger } from '../utils/logger.js'

const SESSION_COOKIE_NAME = 'moatless_session'

/**
 * GitHub OAuth authentication middleware
 * Validates session and injects user information into context
 * Only active when GITHUB_OAUTH_ENABLED=true
 */
export const githubAuth = async (c: Context, next: Next) => {
  // Check if GitHub OAuth is enabled
  if (process.env['GITHUB_OAUTH_ENABLED'] !== 'true') {
    return next()
  }

  // Skip auth for certain endpoints
  const skipPaths = ['/health', '/auth/login', '/auth/github/callback']
  if (skipPaths.some((path) => c.req.path === path)) {
    return next()
  }

  const sessionId = getCookie(c, SESSION_COOKIE_NAME)

  if (!sessionId) {
    throw new HTTPException(401, { message: 'Authentication required' })
  }

  const services = c.get('services')
  const session = await services.auth.getSession(sessionId)

  if (!session) {
    logger.warn(`Invalid session attempt: ${sessionId}`)
    throw new HTTPException(401, { message: 'Session expired or invalid' })
  }

  // Inject session into context for use by route handlers
  c.set('session', session)

  // Continue to next middleware/handler
  await next()
}

/**
 * Optional middleware that only validates session if present
 * Useful for endpoints that work for both authenticated and unauthenticated users
 */
export const optionalGithubAuth = async (c: Context, next: Next) => {
  // Skip if GitHub OAuth is not enabled
  if (process.env['GITHUB_OAUTH_ENABLED'] !== 'true') {
    return next()
  }

  const sessionId = getCookie(c, SESSION_COOKIE_NAME)

  if (sessionId) {
    const services = c.get('services')
    const session = await services.auth.getSession(sessionId)

    if (session) {
      // Inject session into context
      c.set('session', session)
    }
  }

  // Continue regardless of session validity
  await next()
}

/**
 * Helper function to get session from context
 */
export function getSessionFromContext(c: Context): UserSession | undefined {
  return c.get('session')
}

/**
 * Helper function to get GitHub access token from session
 */
export function getGitHubTokenFromContext(c: Context): string | undefined {
  const session = getSessionFromContext(c)
  return session?.accessToken
}
