import type { UserSession } from '@moatless/types'
import type { Context, Next } from 'hono'
import { getCookie } from 'hono/cookie'
import { HTTPException } from 'hono/http-exception'
import { appContainer } from '../container/container'
import { UserRepository } from '../features/auth/db/user.repository'
import { ApiKeyService } from '../features/auth/services/api-key.service'
import { AuthService } from '../features/auth/services/auth.service'
import { SESSION_COOKIE_NAME } from '../features/auth/utils/cookie.utils'
import { logger } from '../utils/logger'

/**
 * Authentication middleware
 * Validates session and injects user information into context
 * Active when AUTH_PROVIDER is set to 'github' or 'auth0'
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Auth middleware requires multiple provider checks
export const auth = async (c: Context, next: Next) => {
  // Check if authentication is enabled
  const authProvider = process.env['AUTH_PROVIDER']

  if (!authProvider) {
    return next()
  }

  // Only require auth for API endpoints
  // Allow all static files and SPA routes to pass through
  if (!c.req.path.startsWith('/api')) {
    return next()
  }

  // Skip auth for auth endpoints (both GitHub and Auth0)
  const skipPaths = [
    '/api/v1/auth/login', // Generic login endpoint
    '/api/v1/auth/logout', // Logout endpoint (allows clearing expired sessions)
    '/api/v1/auth/me', // User info endpoint (handles dev mode internally)
    '/api/v1/auth/github',
    '/api/v1/auth/github/callback',
    '/api/v1/auth/auth0',
    '/api/v1/auth/auth0/callback',
    // External channel webhooks (called by SendGrid, Slack, etc.)
    '/api/v1/channels/email/inbound',
    '/api/v1/integrations/slack/events',
  ]
  if (skipPaths.some((path) => c.req.path === path)) {
    return next()
  }

  // Skip auth for MCP endpoints (server-to-server communication from Claude Code SDK)
  // Pattern: /api/v1/tasks/{taskId}/mcp
  // These are only called by the backend agent process, not from browser
  if (c.req.path.match(/^\/api\/v1\/tasks\/[^/]+\/mcp$/)) {
    return next()
  }

  // Check for API key authentication (Bearer mvk_xxx)
  // API keys can be used for programmatic access to any API endpoint
  const authHeader = c.req.header('Authorization')
  if (authHeader?.startsWith('Bearer mvk_')) {
    const apiKey = authHeader.slice(7) // Remove 'Bearer ' prefix

    try {
      const apiKeyService = appContainer.get(ApiKeyService)
      const keyPayload = await apiKeyService.validateKey(apiKey)

      if (keyPayload) {
        // Get user role from database
        const userRepo = appContainer.get(UserRepository)
        const user = await userRepo.getUser(keyPayload.userId)
        const userRole = user?.role ?? 'user'

        // Create synthetic session from API key
        const syntheticSession: UserSession = {
          id: `apikey-${keyPayload.keyId}`,
          provider: 'api-key',
          userId: keyPayload.userId,
          userLogin: keyPayload.userName || 'api-key',
          userEmail: null,
          userName: keyPayload.userName,
          avatarUrl: null,
          role: userRole, // Include user role
          accessToken: '', // No OAuth token for API key sessions
          createdAt: new Date(),
          expiresAt: keyPayload.expiresAt || new Date(Date.now() + 24 * 60 * 60 * 1000),
          ...(keyPayload.groupId && { groupId: keyPayload.groupId }),
        }
        c.set('session', syntheticSession)
        logger.debug(
          { keyId: keyPayload.keyId, userId: keyPayload.userId },
          'API key authenticated',
        )
        return next()
      }
    } catch (error) {
      logger.warn({ err: error }, 'API key validation error')
    }

    throw new HTTPException(401, { message: 'Invalid API key' })
  }

  const sessionId = getCookie(c, SESSION_COOKIE_NAME)

  if (!sessionId) {
    throw new HTTPException(401, { message: 'Authentication required' })
  }

  const authService = appContainer.get(AuthService)
  const session = await authService.getSession(sessionId)

  if (!session) {
    logger.warn({ sessionId }, 'Invalid session attempt')
    throw new HTTPException(401, { message: 'Session expired or invalid' })
  }

  // Inject session into context for use by route handlers
  c.set('session', session)

  // Continue to next middleware/handler
  await next()
}

// Export githubAuth as an alias for backward compatibility
export const githubAuth = auth

/**
 * Optional middleware that only validates session if present
 * Useful for endpoints that work for both authenticated and unauthenticated users
 */
export const optionalAuth = async (c: Context, next: Next) => {
  // Skip if no authentication is enabled
  const authProvider = process.env['AUTH_PROVIDER']

  if (!authProvider) {
    return next()
  }

  const sessionId = getCookie(c, SESSION_COOKIE_NAME)

  if (sessionId) {
    const authService = appContainer.get(AuthService)
    const session = await authService.getSession(sessionId)

    if (session) {
      // Inject session into context
      c.set('session', session)
    }
  }

  // Continue regardless of session validity
  await next()
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
