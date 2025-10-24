const SESSION_COOKIE_NAME = 'moatless_session'

/**
 * Get session cookie options
 * Ensures consistent cookie attributes across set and delete operations
 * @param maxAge - Optional max age in seconds
 */
export function getSessionCookieOptions(maxAge?: number) {
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
export function getSessionMaxAge(): number {
  return Number.parseInt(process.env['SESSION_MAX_AGE'] || '', 10) || 7 * 24 * 60 * 60
}

export { SESSION_COOKIE_NAME }
