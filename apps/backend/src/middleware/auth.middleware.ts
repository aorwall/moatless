import type { UserSession } from '@moatless/types'
import type { Context, Next } from 'hono'
import { HTTPException } from 'hono/http-exception'
import * as jose from 'jose'
import { logger } from '../utils/logger'

const SKIP_PATHS = [
  '/api/v1/auth/', // All auth routes handled by Rust via Traefik routing
  '/api/v1/channels/email/inbound',
  '/api/v1/integrations/slack/events',
]

function sessionFromHeaders(c: Context): UserSession {
  const userId = c.req.header('X-User-Id') as string
  return {
    id: `forwarded-${userId}`,
    provider: c.req.header('X-User-Provider') || 'unknown',
    userId,
    userLogin: c.req.header('X-User-Login') || '',
    userEmail: null,
    userName: null,
    avatarUrl: null,
    role: (c.req.header('X-User-Role') as 'admin' | 'user') || 'user',
    accessToken: '',
    createdAt: new Date(),
    expiresAt: new Date(Date.now() + 24 * 60 * 60 * 1000),
    ...(c.req.header('X-Group-Id') && { groupId: c.req.header('X-Group-Id') }),
  }
}

async function validateUserToken(c: Context): Promise<void> {
  const userToken = c.req.header('X-User-Token') as string
  const tokenSecret = process.env['TASK_TOKEN_SECRET']
  if (!tokenSecret) {
    logger.error('TASK_TOKEN_SECRET not configured')
    throw new HTTPException(500, { message: 'Server configuration error' })
  }

  const secretKey = new TextEncoder().encode(tokenSecret)
  const { payload } = await jose.jwtVerify(userToken, secretKey)

  const userId = payload['userId']
  const login = payload['login']

  if (typeof userId !== 'string') {
    throw new HTTPException(401, { message: 'Invalid token payload' })
  }

  const taskId = c.req.header('X-Task-Id')

  const syntheticSession: UserSession = {
    id: `user-token-${userId}`,
    provider: 'user-token',
    userId,
    userLogin: typeof login === 'string' ? login : userId,
    userEmail: null,
    userName: null,
    avatarUrl: null,
    role: 'user',
    accessToken: '',
    createdAt: new Date(),
    expiresAt: new Date(Date.now() + 90 * 24 * 60 * 60 * 1000),
  }

  c.set('session', syntheticSession)
  logger.debug({ userId, taskId }, 'User token authenticated')
}

/**
 * Authentication middleware
 *
 * After the auth migration to the Rust backend, this middleware handles:
 * 1. User token auth (X-User-Token) — sandbox containers call Node.js with per-user JWT
 * 2. ForwardAuth headers (X-User-*) — set by Traefik after Rust validates session/API key
 *
 * Session and API key validation is now done by the Rust service via Traefik ForwardAuth.
 * The Rust /api/v1/auth/verify endpoint validates and returns user info as response headers,
 * which Traefik forwards to this backend as request headers.
 */
export const auth = async (c: Context, next: Next) => {
  if (!process.env['AUTH_PROVIDER']) {
    return next()
  }

  if (!c.req.path.startsWith('/api')) {
    return next()
  }

  if (SKIP_PATHS.some((path) => c.req.path.startsWith(path))) {
    return next()
  }

  // 1. User token auth (sandbox containers → Node.js)
  if (c.req.header('X-User-Token')) {
    try {
      await validateUserToken(c)
      return next()
    } catch (error) {
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof jose.errors.JWTExpired) {
        throw new HTTPException(401, { message: 'User token expired' })
      }
      if (error instanceof jose.errors.JWTInvalid) {
        throw new HTTPException(401, { message: 'Invalid user token' })
      }
      logger.warn({ err: error }, 'User token validation error')
      throw new HTTPException(401, { message: 'User token validation failed' })
    }
  }

  // 2. ForwardAuth headers (set by Traefik after Rust validates session/API key)
  if (c.req.header('X-User-Id')) {
    c.set('session', sessionFromHeaders(c))
    return next()
  }

  // 3. Fallback: no valid auth found
  throw new HTTPException(401, { message: 'Authentication required' })
}

// Export githubAuth as an alias for backward compatibility
export const githubAuth = auth

/**
 * Optional middleware that only validates session if present
 * Useful for endpoints that work for both authenticated and unauthenticated users
 */
export const optionalAuth = (c: Context, next: Next) => {
  if (!process.env['AUTH_PROVIDER']) {
    return next()
  }

  if (c.req.header('X-User-Id')) {
    c.set('session', sessionFromHeaders(c))
  }

  return next()
}

// Export optionalGithubAuth as an alias for backward compatibility
export const optionalGithubAuth = optionalAuth

/**
 * Helper function to get session from context
 */
export function getSessionFromContext(c: Context): UserSession | undefined {
  return c.get('session')
}

/**
 * Helper function to get GitHub access token from session
 * Returns token only for GitHub OAuth sessions
 * For other auth providers (Auth0, etc.), returns undefined to allow PAT fallback
 */
export function getGitHubTokenFromContext(c: Context): string | undefined {
  const session = getSessionFromContext(c)

  // Only return session token if authenticated via GitHub OAuth
  // This ensures Auth0 sessions fall back to PAT lookup
  if (session?.provider === 'github') {
    const token = session.accessToken
    // Guard against empty string tokens - treat them as missing
    return token?.trim() ? token : undefined
  }

  return undefined
}
