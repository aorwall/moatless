import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  authAuth0CallbackRoute,
  authLoginRoute,
  getCurrentUserRoute,
  initiateAuth0OAuthRoute,
  initiateAuthLoginRoute,
  initiateGitHubOAuthRoute,
  logoutGetRoute,
  logoutRoute,
} from '@moatless/api-contract/auth'
import { deleteCookie, getCookie, setCookie } from 'hono/cookie'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app.js'
import { getSessionCookieOptions, SESSION_COOKIE_NAME } from '../utils/cookie.utils.js'

/**
 * Register authentication routes
 */
export function registerAuthRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * GET /auth/login
   * Generic login endpoint that redirects to the configured auth provider
   */
  app.openapi(initiateAuthLoginRoute, (c) => {
    const authProvider = process.env['AUTH_PROVIDER']

    if (!authProvider) {
      throw new HTTPException(503, {
        message: 'Authentication is not enabled. Set AUTH_PROVIDER environment variable.',
      })
    }

    // Redirect to the appropriate provider endpoint
    if (authProvider === 'github') {
      return c.redirect('/api/v1/auth/github')
    }

    if (authProvider === 'auth0') {
      return c.redirect('/api/v1/auth/auth0')
    }

    throw new HTTPException(500, {
      message: `Invalid AUTH_PROVIDER: ${authProvider}`,
    })
  })

  /**
   * GET /auth/github
   * Initiates GitHub OAuth flow
   */
  app.openapi(initiateGitHubOAuthRoute, (c) => {
    const services = c.get('services')

    // Generate CSRF state token
    const state = services.auth.generateOAuthState()

    // Generate GitHub OAuth URL using provider abstraction
    const authUrl = services.auth.generateAuthUrl('github', state)

    if (!authUrl) {
      throw new HTTPException(500, {
        message: 'Failed to generate OAuth URL - check GitHub OAuth configuration',
      })
    }

    // Store state in cookie for verification on callback
    setCookie(c, 'oauth_state', state, {
      httpOnly: true,
      secure: process.env['NODE_ENV'] === 'production',
      maxAge: 600, // 10 minutes
      sameSite: 'Lax',
      path: '/',
    })

    // Redirect to GitHub OAuth page
    return c.redirect(authUrl)
  })

  /**
   * GET /auth/login
   * OAuth callback - exchanges code for session (GitHub)
   */
  app.openapi(authLoginRoute, async (c) => {
    const { code, state } = c.req.valid('query')
    const storedState = getCookie(c, 'oauth_state')
    const services = c.get('services')

    try {
      // Handle callback via provider abstraction
      const result = await services.auth.handleCallback('github', code, state, storedState)

      // Clear the state cookie
      deleteCookie(c, 'oauth_state')

      // Set session cookie
      setCookie(
        c,
        SESSION_COOKIE_NAME,
        result.sessionId,
        getSessionCookieOptions(result.sessionMaxAge),
      )

      // Redirect to frontend
      return c.redirect(result.redirectUrl)
    } catch (error) {
      // Clear the state cookie on error
      deleteCookie(c, 'oauth_state')

      const message = error instanceof Error ? error.message : 'Authentication failed'
      const status = message.includes('Invalid state') ? 400 : 500
      throw new HTTPException(status, { message })
    }
  })

  /**
   * GET /auth/auth0
   * Initiates Auth0 OAuth flow
   */
  app.openapi(initiateAuth0OAuthRoute, (c) => {
    const services = c.get('services')

    // Generate CSRF state token
    const state = services.auth.generateOAuthState()

    // Generate Auth0 OAuth URL using provider abstraction
    const authUrl = services.auth.generateAuthUrl('auth0', state)

    if (!authUrl) {
      throw new HTTPException(500, {
        message: 'Failed to generate OAuth URL - check Auth0 configuration',
      })
    }

    // Store state in cookie for verification on callback
    setCookie(c, 'oauth_state', state, {
      httpOnly: true,
      secure: process.env['NODE_ENV'] === 'production',
      maxAge: 600, // 10 minutes
      sameSite: 'Lax',
      path: '/',
    })

    // Redirect to Auth0 OAuth page
    return c.redirect(authUrl)
  })

  /**
   * GET /auth/auth0/callback
   * OAuth callback - exchanges code for session (Auth0)
   */
  app.openapi(authAuth0CallbackRoute, async (c) => {
    const { code, state } = c.req.valid('query')
    const storedState = getCookie(c, 'oauth_state')
    const services = c.get('services')

    try {
      // Handle callback via provider abstraction
      const result = await services.auth.handleCallback('auth0', code, state, storedState)

      // Clear the state cookie
      deleteCookie(c, 'oauth_state')

      // Set session cookie
      setCookie(
        c,
        SESSION_COOKIE_NAME,
        result.sessionId,
        getSessionCookieOptions(result.sessionMaxAge),
      )

      // Redirect to frontend
      return c.redirect(result.redirectUrl)
    } catch (error) {
      // Clear the state cookie on error
      deleteCookie(c, 'oauth_state')

      const message = error instanceof Error ? error.message : 'Authentication failed'
      const status = message.includes('Invalid state') ? 400 : 500
      throw new HTTPException(status, { message })
    }
  })

  /**
   * GET /auth/me
   * Get current authenticated user (with dev mode support)
   */
  app.openapi(getCurrentUserRoute, async (c) => {
    const sessionId = getCookie(c, SESSION_COOKIE_NAME)
    const services = c.get('services')

    try {
      const response = await services.auth.getCurrentUserSession(sessionId)
      return c.json(response)
    } catch (error) {
      // Clear invalid session cookie
      if (sessionId) {
        deleteCookie(c, SESSION_COOKIE_NAME)
      }

      const message = error instanceof Error ? error.message : 'Authentication error'
      const status =
        message.includes('Not authenticated') || message.includes('expired') ? 401 : 500
      throw new HTTPException(status, { message })
    }
  })

  /**
   * POST /auth/logout
   * Logout and destroy session
   * For Auth0 users, redirects to Auth0 logout to clear Auth0 session
   * For GitHub users (or no session), returns JSON response
   */
  app.openapi(logoutRoute, async (c) => {
    const sessionId = getCookie(c, SESSION_COOKIE_NAME)
    const services = c.get('services')

    // Handle logout via service
    const result = await services.auth.handleLogout(sessionId)

    // Delete cookie
    deleteCookie(c, SESSION_COOKIE_NAME, getSessionCookieOptions())

    // For Auth0 users, redirect to Auth0 logout
    if (result.logoutUrl) {
      return c.redirect(result.logoutUrl)
    }

    // For GitHub users or if Auth0 logout URL generation failed
    return c.json({ message: 'Logged out successfully' })
  })

  /**
   * GET /auth/logout
   * Logout and destroy session (GET version for browser navigation)
   * For Auth0 users, redirects to Auth0 logout to clear Auth0 session
   * For GitHub users (or no session), redirects to home page
   */
  app.openapi(logoutGetRoute, async (c) => {
    const sessionId = getCookie(c, SESSION_COOKIE_NAME)
    const services = c.get('services')

    // Handle logout via service
    const result = await services.auth.handleLogout(sessionId)

    // Delete cookie
    deleteCookie(c, SESSION_COOKIE_NAME, getSessionCookieOptions())

    // For Auth0 users, redirect to Auth0 logout
    if (result.logoutUrl) {
      return c.redirect(result.logoutUrl)
    }

    // For GitHub users or if Auth0 logout URL generation failed
    return c.json({ message: 'Logged out successfully' })
  })
}
