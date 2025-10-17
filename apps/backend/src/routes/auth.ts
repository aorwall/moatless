import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  authAuth0CallbackRoute,
  authLoginRoute,
  getCurrentUserRoute,
  initiateAuth0OAuthRoute,
  initiateAuthLoginRoute,
  initiateGitHubOAuthRoute,
  logoutRoute,
} from '@moatless/api-contract/auth'
import { deleteCookie, getCookie, setCookie } from 'hono/cookie'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { logger } from '../utils/logger.js'

const SESSION_COOKIE_NAME = 'moatless_session'

/**
 * Get session cookie options
 * Ensures consistent cookie attributes across set and delete operations
 * @param maxAge - Optional max age in seconds
 */
function getSessionCookieOptions(maxAge?: number) {
  return {
    httpOnly: true,
    secure: process.env['NODE_ENV'] === 'production',
    sameSite: 'Lax' as const,
    path: '/',
    ...(maxAge !== undefined && { maxAge }),
  }
}

/**
 * Get session max age from environment or default to 7 days
 */
function getSessionMaxAge(): number {
  return Number.parseInt(process.env['SESSION_MAX_AGE'] || '', 10) || 7 * 24 * 60 * 60
}

/**
 * Determine user's group ID if group-based storage is enabled
 * @param services - Service container with auth services
 * @param provider - Authentication provider ('github' or 'auth0')
 * @param accessToken - OAuth access token (used for GitHub)
 * @param idToken - Optional ID token (used for Auth0)
 * @throws HTTPException if user is not assigned to a group when group storage is required
 */
async function determineUserGroup(
  services: AppBindings['Variables']['services'],
  provider: 'github' | 'auth0',
  accessToken: string,
  idToken?: string,
): Promise<string | undefined> {
  const storageType = process.env['STORAGE_TYPE']

  if (storageType !== 'group') {
    return undefined
  }

  // Handle Auth0 provider with ID token
  if (provider === 'auth0') {
    if (!idToken) {
      throw new HTTPException(500, {
        message: 'ID token required for Auth0 group determination',
      })
    }

    // Extract group from Auth0 ID token
    const groupId = services.auth0.getPrimaryGroup(idToken)

    if (!groupId) {
      logger.warn('Auth0 user has no groups assigned in ID token')
      throw new HTTPException(403, {
        message:
          'Access denied: You are not assigned to any group. Please contact an administrator to add you to a group.',
      })
    }

    logger.info(`Auth0 user assigned to group: ${groupId}`)
    return groupId
  }

  // Handle GitHub provider with settings-based lookup
  const user = await services.github.getAuthenticatedUser(accessToken)

  if (!user) {
    throw new HTTPException(500, { message: 'Failed to fetch user information' })
  }

  const githubUser = user as { id: number; login: string }

  // Load group configuration from settings
  const settings = await services.settings.getSettings()
  const userGroups = settings?.settings?.userGroups

  if (!userGroups) {
    logger.warn('Group-based storage enabled but no userGroups configuration found in settings')
    throw new HTTPException(500, {
      message: 'Group configuration not found. Please contact an administrator.',
    })
  }

  // Look up user's group
  const groupId = userGroups[githubUser.id.toString()]

  if (!groupId) {
    logger.warn(`User ${githubUser.login} (ID: ${githubUser.id}) not assigned to any group`)
    throw new HTTPException(403, {
      message: 'Access denied: You are not assigned to any group. Please contact an administrator.',
    })
  }

  logger.info(`GitHub user ${githubUser.login} assigned to group: ${groupId}`)
  return groupId
}

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

    // Generate GitHub OAuth URL
    const authUrl = services.auth.generateGitHubAuthUrl(state)

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
      throw new HTTPException(500, {
        message: 'Failed to exchange code for token',
      })
    }

    // Determine group ID if group-based access control is enabled
    const groupId = await determineUserGroup(services, 'github', tokenResponse.access_token)

    // Create session with optional group ID
    const sessionId = await services.auth.createSession(
      tokenResponse.access_token,
      tokenResponse.token_type,
      tokenResponse.scope,
      groupId,
    )

    if (!sessionId) {
      throw new HTTPException(500, { message: 'Failed to create session' })
    }

    // Set session cookie
    const maxAge = getSessionMaxAge()
    setCookie(c, SESSION_COOKIE_NAME, sessionId, getSessionCookieOptions(maxAge))

    // Redirect to frontend homepage (session cookie is already set)
    const frontendUrl = process.env['FRONTEND_URL'] || 'http://localhost:3000'
    return c.redirect(frontendUrl)
  })

  /**
   * GET /auth/auth0
   * Initiates Auth0 OAuth flow
   */
  app.openapi(initiateAuth0OAuthRoute, (c) => {
    const services = c.get('services')

    // Generate CSRF state token
    const state = services.auth.generateOAuthState()

    // Generate Auth0 OAuth URL
    const authUrl = services.auth.generateAuth0AuthUrl(state)

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
   * OAuth callback - exchanges code for session
   */
  app.openapi(authAuth0CallbackRoute, async (c) => {
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

    // Exchange code for tokens
    const tokenResponse = await services.auth.exchangeAuth0CodeForToken(code)

    if (!tokenResponse) {
      throw new HTTPException(500, {
        message: 'Failed to exchange code for token',
      })
    }

    // Determine group ID if group-based access control is enabled
    const groupId = await determineUserGroup(
      services,
      'auth0',
      tokenResponse.access_token,
      tokenResponse.id_token,
    )

    // Create Auth0 session
    const sessionId = await services.auth.createAuth0Session(
      tokenResponse.access_token,
      tokenResponse.id_token,
      tokenResponse.token_type,
      tokenResponse.scope,
      groupId,
    )

    if (!sessionId) {
      throw new HTTPException(500, { message: 'Failed to create session' })
    }

    // Set session cookie
    const maxAge = getSessionMaxAge()
    setCookie(c, SESSION_COOKIE_NAME, sessionId, getSessionCookieOptions(maxAge))

    // Redirect to frontend homepage (session cookie is already set)
    const frontendUrl = process.env['FRONTEND_URL'] || 'http://localhost:3000'
    return c.redirect(frontendUrl)
  })

  /**
   * GET /auth/me
   * Get current authenticated user (with dev mode support)
   */
  app.openapi(getCurrentUserRoute, async (c) => {
    // In development mode (when no auth is enabled), return null user
    const authProvider = process.env['AUTH_PROVIDER']

    if (!authProvider) {
      return c.json({
        user: null,
        expiresAt: null,
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

    // Delete cookie with same attributes as when it was set (without maxAge)
    deleteCookie(c, SESSION_COOKIE_NAME, getSessionCookieOptions())

    return c.json({ message: 'Logged out successfully' })
  })
}
