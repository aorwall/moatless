import type { Context, Next } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { logger } from '../utils/logger.js'

/**
 * Basic authentication middleware
 * Can be configured via environment variables:
 * - BASIC_AUTH_USER: Username (default: admin)
 * - BASIC_AUTH_PASS: Password (required if auth is enabled)
 * - BASIC_AUTH_ENABLED: Set to 'true' to enable auth
 */
export const basicAuth = async (c: Context, next: Next) => {
  // Check if basic auth is enabled
  if (process.env.BASIC_AUTH_ENABLED !== 'true') {
    return next()
  }

  // Skip auth for health endpoint
  if (c.req.path === '/health') {
    return next()
  }

  const username = process.env.BASIC_AUTH_USER || 'admin'
  const password = process.env.BASIC_AUTH_PASS

  if (!password) {
    logger.error('Basic auth is enabled but BASIC_AUTH_PASS is not set')
    throw new HTTPException(500, { message: 'Server configuration error' })
  }

  const authHeader = c.req.header('Authorization')

  if (!authHeader) {
    c.header('WWW-Authenticate', 'Basic realm="Moatless Vibe"')
    throw new HTTPException(401, { message: 'Authentication required' })
  }

  const authValue = authHeader.split(' ')[1]
  if (!authValue) {
    c.header('WWW-Authenticate', 'Basic realm="Moatless Vibe"')
    throw new HTTPException(401, { message: 'Invalid authentication format' })
  }

  const decoded = atob(authValue)
  const [providedUser, providedPass] = decoded.split(':')

  if (providedUser !== username || providedPass !== password) {
    logger.warn(
      `Failed authentication attempt from ${c.req.header('x-forwarded-for') || c.req.header('x-real-ip') || 'unknown'}`,
    )
    c.header('WWW-Authenticate', 'Basic realm="Moatless Vibe"')
    throw new HTTPException(401, { message: 'Invalid credentials' })
  }

  // Authentication successful
  await next()
}

/**
 * Alternative: Token-based authentication middleware
 * Can be configured via environment variables:
 * - AUTH_TOKEN: Secret token for authentication
 * - AUTH_TOKEN_ENABLED: Set to 'true' to enable token auth
 */
export const tokenAuth = async (c: Context, next: Next) => {
  // Check if token auth is enabled
  if (process.env.AUTH_TOKEN_ENABLED !== 'true') {
    return next()
  }

  // Skip auth for health endpoint
  if (c.req.path === '/health') {
    return next()
  }

  const token = process.env.AUTH_TOKEN

  if (!token) {
    logger.error('Token auth is enabled but AUTH_TOKEN is not set')
    throw new HTTPException(500, { message: 'Server configuration error' })
  }

  // Check for token in various places
  const providedToken =
    c.req.header('Authorization')?.replace('Bearer ', '') ||
    c.req.query('token') ||
    c.req.header('X-Auth-Token')

  if (!providedToken) {
    throw new HTTPException(401, { message: 'Authentication token required' })
  }

  if (providedToken !== token) {
    logger.warn(
      `Failed token authentication attempt from ${c.req.header('x-forwarded-for') || c.req.header('x-real-ip') || 'unknown'}`,
    )
    throw new HTTPException(401, { message: 'Invalid authentication token' })
  }

  // Authentication successful
  await next()
}
