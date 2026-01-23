import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { getSessionCookieOptions } from '../cookie.utils'

describe('Cookie Utils', () => {
  let originalEnv: NodeJS.ProcessEnv

  beforeEach(() => {
    // Save original environment
    originalEnv = { ...process.env }
  })

  afterEach(() => {
    // Restore original environment
    process.env = originalEnv
  })

  describe('getSessionCookieOptions', () => {
    it('should return basic options without domain when hostname routing is disabled', () => {
      process.env['USE_HOSTNAME_ROUTING'] = undefined
      process.env['PROXY_DOMAIN'] = undefined

      const options = getSessionCookieOptions()

      expect(options).toEqual({
        httpOnly: true,
        secure: false,
        sameSite: 'Lax',
        path: '/',
      })
      expect(options).not.toHaveProperty('domain')
    })

    it('should include domain when hostname routing is enabled with valid domain', () => {
      process.env['USE_HOSTNAME_ROUTING'] = 'true'
      process.env['PROXY_DOMAIN'] = 'proxy.moatless.xyz'

      const options = getSessionCookieOptions()

      expect(options).toHaveProperty('domain', '.moatless.xyz')
    })

    it('should extract base domain from subdomain', () => {
      process.env['USE_HOSTNAME_ROUTING'] = 'true'
      process.env['PROXY_DOMAIN'] = 'preview.proxy.moatless.xyz'

      const options = getSessionCookieOptions()

      // Should extract last two parts: moatless.xyz
      expect(options).toHaveProperty('domain', '.moatless.xyz')
    })

    it('should not include domain for localhost', () => {
      process.env['USE_HOSTNAME_ROUTING'] = 'true'
      process.env['PROXY_DOMAIN'] = 'localhost'

      const options = getSessionCookieOptions()

      expect(options).not.toHaveProperty('domain')
    })

    it('should not include domain for 127.0.0.1', () => {
      process.env['USE_HOSTNAME_ROUTING'] = 'true'
      process.env['PROXY_DOMAIN'] = '127.0.0.1'

      const options = getSessionCookieOptions()

      expect(options).not.toHaveProperty('domain')
    })

    it('should include maxAge when provided', () => {
      process.env['USE_HOSTNAME_ROUTING'] = 'true'
      process.env['PROXY_DOMAIN'] = 'proxy.moatless.xyz'

      const options = getSessionCookieOptions(3600)

      expect(options).toHaveProperty('maxAge', 3600)
      expect(options).toHaveProperty('domain', '.moatless.xyz')
    })

    it('should set secure flag in production', () => {
      process.env['NODE_ENV'] = 'production'
      process.env['USE_HOSTNAME_ROUTING'] = 'true'
      process.env['PROXY_DOMAIN'] = 'proxy.moatless.xyz'

      const options = getSessionCookieOptions()

      expect(options).toHaveProperty('secure', true)
      expect(options).toHaveProperty('domain', '.moatless.xyz')
    })

    it('should not include domain when hostname routing is enabled but domain is not set', () => {
      process.env['USE_HOSTNAME_ROUTING'] = 'true'
      process.env['PROXY_DOMAIN'] = undefined

      const options = getSessionCookieOptions()

      expect(options).not.toHaveProperty('domain')
    })

    it('should handle single-part domain gracefully', () => {
      process.env['USE_HOSTNAME_ROUTING'] = 'true'
      process.env['PROXY_DOMAIN'] = 'localhost'

      const options = getSessionCookieOptions()

      // Should not include domain for single-part domains
      expect(options).not.toHaveProperty('domain')
    })

    it('should include domain with leading dot for wildcard subdomain matching', () => {
      process.env['USE_HOSTNAME_ROUTING'] = 'true'
      process.env['PROXY_DOMAIN'] = 'proxy.example.com'

      const options = getSessionCookieOptions()

      // Domain should have leading dot for wildcard matching
      expect(options.domain).toMatch(/^\..+/)
      expect(options).toHaveProperty('domain', '.example.com')
    })
  })
})
