import type { IncomingMessage } from 'node:http'
import type { UserSession } from '@moatless/types'
import { appContainer } from '../../../container/container.js'
import type { Logger } from '../../../utils/logger.js'
import { AuthService } from '../../auth/services/auth.service.js'

/**
 * Parse cookie header to extract a specific cookie value
 */
export function getCookieValue(
  cookieHeader: string | undefined,
  cookieName: string,
): string | null {
  if (!cookieHeader) {
    return null
  }

  const cookies = cookieHeader.split(';').map((c) => c.trim())
  for (const cookie of cookies) {
    const [name, value] = cookie.split('=')
    if (name === cookieName) {
      return value || null
    }
  }
  return null
}

/**
 * Validate session from request cookies
 * Returns:
 * - UserSession object if valid session found
 * - undefined if authentication is disabled
 * - null if authentication is enabled but no valid session found
 */
export async function validateProxySession(
  req: IncomingMessage,
  logger: Logger,
): Promise<UserSession | null | undefined> {
  // Check if authentication is enabled
  const authProvider = process.env['AUTH_PROVIDER']
  if (!authProvider || authProvider === 'disabled') {
    // Auth disabled, allow all requests
    logger.debug('Authentication disabled, allowing proxy request')
    return undefined
  }

  // Extract session cookie
  const cookieHeader = req.headers.cookie
  const sessionId = getCookieValue(cookieHeader, 'moatless_session')

  if (!sessionId) {
    logger.debug('No session cookie found in proxy request')
    return null
  }

  try {
    // Validate session using AuthService
    const authService = appContainer.get(AuthService)
    const session = await authService.getSession(sessionId)

    if (session) {
      logger.debug('Valid session found for proxy request', {
        userId: session.userId,
        userLogin: session.userLogin,
      })
      return session
    }

    logger.debug('Invalid or expired session in proxy request')
    return null
  } catch (error) {
    logger.error('Error validating session for proxy request', error as Error)
    return null
  }
}

/**
 * Check if authentication is required and validate the request
 * Returns true if the request should proceed, false if it should be rejected with 401
 */
export async function isProxyRequestAuthorized(
  req: IncomingMessage,
  logger: Logger,
): Promise<boolean> {
  const authProvider = process.env['AUTH_PROVIDER']
  if (!authProvider || authProvider === 'disabled') {
    return true
  }

  const session = await validateProxySession(req, logger)

  // session can be:
  // - UserSession object: valid session, authorize request
  // - undefined: auth is disabled, authorize request
  // - null: auth enabled but no valid session, reject request
  return session !== null
}
