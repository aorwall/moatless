import type { OpenAPIHono } from '@hono/zod-openapi'
import { authLoginRoute, getCurrentUserRoute, logoutRoute } from '@moatless/api-contract/auth'
import { deleteCookie, getCookie, setCookie } from 'hono/cookie'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { logger } from '../utils/logger.js'

const SESSION_COOKIE_NAME = 'moatless_session'

/**
 * Register authentication routes
 */
export function registerAuthRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * GET /auth/login
   * OAuth callback - exchanges code for session
   */
  app.openapi(authLoginRoute, async (c) => {
    const { code, state } = c.req.valid('query')

    // Verify state parameter for CSRF protection
    const storedState = getCookie(c, 'oauth_state')
    if (state && state !== storedState) {
      logger.warn('OAuth state mismatch - possible CSRF attack')
      throw new HTTPException(400, { message: 'Invalid state parameter' })
    }

    // Clear the state cookie
    deleteCookie(c, 'oauth_state')

    const services = c.get('services')

    // Exchange code for access token
    const tokenResponse = await services.auth.exchangeCodeForToken(code)

    if (!tokenResponse) {
      throw new HTTPException(500, { message: 'Failed to exchange code for token' })
    }

    // Create session
    const sessionId = await services.auth.createSession(
      tokenResponse.access_token,
      tokenResponse.token_type,
      tokenResponse.scope,
    )

    if (!sessionId) {
      throw new HTTPException(500, { message: 'Failed to create session' })
    }

    // Set session cookie
    const maxAge = Number.parseInt(process.env['SESSION_MAX_AGE'] || '', 10) || 7 * 24 * 60 * 60 // 7 days
    setCookie(c, SESSION_COOKIE_NAME, sessionId, {
      httpOnly: true,
      secure: process.env['NODE_ENV'] === 'production',
      maxAge,
      sameSite: 'Lax',
      path: '/',
    })

    // Redirect to frontend
    const frontendUrl = process.env['FRONTEND_URL'] || 'http://localhost:3000'
    return c.redirect(`${frontendUrl}/auth/callback?success=true`)
  })

  /**
   * GET /auth/me
   * Get current authenticated user (with dev mode support)
   */
  app.openapi(getCurrentUserRoute, async (c) => {
    // In development mode (when GitHub OAuth is disabled), return a mock dev user
    if (process.env['GITHUB_OAUTH_ENABLED'] !== 'true') {
      return c.json({
        user: {
          id: 1,
          login: 'dev-user',
          email: 'dev@localhost',
          name: 'Development User',
          avatarUrl: 'https://avatars.githubusercontent.com/u/1',
        },
        expiresAt: '2099-12-31T23:59:59.999Z',
      })
    }

    // Production mode - require valid session
    const sessionId = getCookie(c, SESSION_COOKIE_NAME)

    if (!sessionId) {
      throw new HTTPException(401, { message: 'Not authenticated' })
    }

    const services = c.get('services')
    const session = await services.auth.getSession(sessionId)

    if (!session) {
      // Clear invalid session cookie
      deleteCookie(c, SESSION_COOKIE_NAME)
      throw new HTTPException(401, { message: 'Session expired or invalid' })
    }

    return c.json(services.auth.sessionToResponse(session))
  })

  /**
   * POST /auth/logout
   * Logout and destroy session
   */
  app.openapi(logoutRoute, async (c) => {
    const sessionId = getCookie(c, SESSION_COOKIE_NAME)

    if (sessionId) {
      const services = c.get('services')
      await services.auth.deleteSession(sessionId)
    }

    deleteCookie(c, SESSION_COOKIE_NAME)

    return c.json({ message: 'Logged out successfully' })
  })
}
