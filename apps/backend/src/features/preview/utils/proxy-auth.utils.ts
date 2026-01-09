import type { IncomingMessage } from 'node:http'
import type { UserSession } from '@moatless/types'
import { appContainer } from '../../../container/container.js'
import type { Logger } from '../../../utils/logger.js'
import { UserRepository } from '../../auth/db/user.repository.js'
import { ApiKeyService } from '../../auth/services/api-key.service.js'
import { AuthService } from '../../auth/services/auth.service.js'
import { SESSION_COOKIE_NAME } from '../../auth/utils/cookie.utils.js'

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
  const sessionId = getCookieValue(cookieHeader, SESSION_COOKIE_NAME)

  if (!sessionId) {
    logger.debug('No session cookie found in proxy request')
    return null
  }

  try {
    // Validate session using AuthService
    const authService = appContainer.get(AuthService)
    const session = await authService.getSession(sessionId)

    if (session) {
      logger.debug(
        {
          userId: session.userId,
          userLogin: session.userLogin,
        },
        'Valid session found for proxy request',
      )
      return session
    }

    logger.debug('Invalid or expired session in proxy request')
    return null
  } catch (error) {
    logger.error({ err: error as Error }, 'Error validating session for proxy request')
    return null
  }
}

/**
 * Validate API key from Authorization header
 * Returns:
 * - UserSession object if valid API key found
 * - null if no valid API key found
 */
export async function validateProxyApiKey(
  req: IncomingMessage,
  logger: Logger,
): Promise<UserSession | null> {
  const authHeader = req.headers.authorization

  // Check for Bearer token with mvk_ prefix (API key format)
  if (!authHeader) {
    logger.info('No Authorization header found in proxy request')
    return null
  }

  if (!authHeader.startsWith('Bearer mvk_')) {
    logger.info(
      { authHeaderLength: authHeader.length, authHeaderPrefix: authHeader.substring(0, 50) },
      'Authorization header present but not API key format (Bearer mvk_)',
    )
    return null
  }

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
        role: userRole,
        accessToken: '',
        createdAt: new Date(),
        expiresAt: keyPayload.expiresAt || new Date(Date.now() + 24 * 60 * 60 * 1000),
        ...(keyPayload.groupId && { groupId: keyPayload.groupId }),
      }

      logger.info(
        { keyId: keyPayload.keyId, userId: keyPayload.userId },
        'Valid API key found for proxy request',
      )
      return syntheticSession
    }

    logger.info('Invalid API key in proxy request - key not found or expired')
    return null
  } catch (error) {
    logger.error({ err: error as Error }, 'Error validating API key for proxy request')
    return null
  }
}

/**
 * Check if authentication is required and validate the request
 * Supports both session cookies and API key authentication
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

  // First, try API key authentication (Authorization: Bearer mvk_xxx)
  const apiKeySession = await validateProxyApiKey(req, logger)
  if (apiKeySession) {
    return true
  }

  // Fall back to session cookie authentication
  const session = await validateProxySession(req, logger)

  // session can be:
  // - UserSession object: valid session, authorize request
  // - undefined: auth is disabled, authorize request
  // - null: auth enabled but no valid session, reject request
  return session !== null
}
