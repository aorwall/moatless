import { oauthCallbackQuerySchema, sessionResponseSchema } from '@moatless/types'
import type { Hono } from 'hono'
import { deleteCookie, getCookie, setCookie } from 'hono/cookie'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'
import { logger } from '../utils/logger.js'

const SESSION_COOKIE_NAME = 'moatless_session'

/**
 * Register authentication routes
 */
export async function registerAuthRoutes(app: Hono<AppBindings>, prefix: string) {
  /**
   * GET /auth/login
   * Redirect to GitHub OAuth authorization page
   */
  app.get(
    `${prefix}/login`,
    describeRoute({
      description: 'Initiate GitHub OAuth login flow',
      responses: {
        302: {
          description: 'Redirect to GitHub authorization page',
        },
        500: {
          description: 'Server configuration error',
        },
      },
    }),
    (c) => {
      const clientId = process.env.GITHUB_CLIENT_ID
      const callbackUrl = process.env.GITHUB_CALLBACK_URL

      if (!clientId || !callbackUrl) {
        logger.error('GitHub OAuth not properly configured')
        throw new HTTPException(500, { message: 'OAuth not configured' })
      }

      const services = c.get('services')
      const state = services.auth.generateOAuthState()

      // Store state in a short-lived cookie for CSRF protection
      setCookie(c, 'oauth_state', state, {
        httpOnly: true,
        secure: process.env.NODE_ENV === 'production',
        maxAge: 600, // 10 minutes
        sameSite: 'Lax',
      })

      // Redirect to GitHub OAuth
      const authUrl = new URL('https://github.com/login/oauth/authorize')
      authUrl.searchParams.set('client_id', clientId)
      authUrl.searchParams.set('redirect_uri', callbackUrl)
      authUrl.searchParams.set('scope', 'repo user:email')
      authUrl.searchParams.set('state', state)

      return c.redirect(authUrl.toString())
    },
  )

  /**
   * GET /auth/github/callback
   * Handle GitHub OAuth callback
   */
  app.get(
    `${prefix}/github/callback`,
    describeRoute({
      description: 'Handle GitHub OAuth callback',
      responses: {
        302: {
          description: 'Redirect to frontend with session',
        },
        400: {
          description: 'Invalid OAuth callback parameters',
        },
        500: {
          description: 'Failed to create session',
        },
      },
    }),
    async (c) => {
      const queryParams = c.req.query()
      const { code, state } = oauthCallbackQuerySchema.parse(queryParams)

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
      const maxAge = Number.parseInt(process.env.SESSION_MAX_AGE || '', 10) || 7 * 24 * 60 * 60 // 7 days
      setCookie(c, SESSION_COOKIE_NAME, sessionId, {
        httpOnly: true,
        secure: process.env.NODE_ENV === 'production',
        maxAge,
        sameSite: 'Lax',
        path: '/',
      })

      // Redirect to frontend
      const frontendUrl = process.env.FRONTEND_URL || 'http://localhost:3000'
      return c.redirect(`${frontendUrl}/auth/callback?success=true`)
    },
  )

  /**
   * GET /auth/me
   * Get current authenticated user
   */
  app.get(
    `${prefix}/me`,
    describeRoute({
      description: 'Get current authenticated user session',
      responses: {
        200: {
          description: 'Current user session',
          content: {
            'application/json': {
              schema: resolver(sessionResponseSchema),
            },
          },
        },
        401: {
          description: 'Not authenticated',
        },
      },
    }),
    async (c) => {
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
    },
  )

  /**
   * POST /auth/logout
   * Logout and destroy session
   */
  app.post(
    `${prefix}/logout`,
    describeRoute({
      description: 'Logout and destroy session',
      responses: {
        200: {
          description: 'Successfully logged out',
        },
      },
    }),
    async (c) => {
      const sessionId = getCookie(c, SESSION_COOKIE_NAME)

      if (sessionId) {
        const services = c.get('services')
        await services.auth.deleteSession(sessionId)
      }

      deleteCookie(c, SESSION_COOKIE_NAME)

      return c.json({ message: 'Logged out successfully' })
    },
  )
}
