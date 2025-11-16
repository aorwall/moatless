const SESSION_COOKIE_NAME = 'moatless_session'

/**
 * Get the cookie domain for cross-subdomain cookie sharing
 * Returns the base domain with a leading dot when hostname routing is enabled
 * Returns undefined for localhost to avoid cookie issues in development
 */
function getCookieDomain(): string | undefined {
  const useHostnameRouting = process.env['USE_HOSTNAME_ROUTING'] === 'true'
  const proxyDomain = process.env['PROXY_DOMAIN']

  // Only set domain when using hostname routing with a proper domain
  if (useHostnameRouting && proxyDomain) {
    // Don't set domain for localhost
    if (proxyDomain.includes('localhost') || proxyDomain.includes('127.0.0.1')) {
      return undefined
    }

    // Extract base domain from proxy domain
    // e.g., "proxy.moatless.ai" → ".moatless.ai"
    // e.g., "moatless.ai" → ".moatless.ai"
    const parts = proxyDomain.split('.')
    if (parts.length >= 2) {
      // Return the last two parts with a leading dot for wildcard subdomain matching
      return `.${parts.slice(-2).join('.')}`
    }
  }

  return undefined
}

/**
 * Get session cookie options
 * Ensures consistent cookie attributes across set and delete operations
 * @param maxAge - Optional max age in seconds
 */
export function getSessionCookieOptions(maxAge?: number) {
  const domain = getCookieDomain()

  return {
    httpOnly: true,
    secure: process.env['NODE_ENV'] === 'production',
    sameSite: 'Lax' as const,
    path: '/',
    ...(maxAge !== undefined && { maxAge }),
    ...(domain && { domain }),
  }
}

/**
 * Get session max age from environment or default to 7 days
 */
export function getSessionMaxAge(): number {
  return Number.parseInt(process.env['SESSION_MAX_AGE'] || '', 10) || 7 * 24 * 60 * 60
}

export { SESSION_COOKIE_NAME }
