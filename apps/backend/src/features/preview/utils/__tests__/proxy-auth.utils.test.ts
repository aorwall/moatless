import type { IncomingMessage } from 'node:http'
import type { UserSession } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { logger } from '../../../../utils/logger'
import { getCookieValue, isProxyRequestAuthorized, validateProxySession } from '../proxy-auth.utils'

// Mock the container module
const mockAuthService = {
  getSession: vi.fn(),
}

vi.mock('../../../../container/container.js', () => ({
  appContainer: {
    get: vi.fn(() => mockAuthService),
  },
}))

describe('Proxy Authentication Utils', () => {
  const mockLogger = logger.child({ service: 'test' })
  let originalAuthProvider: string | undefined

  beforeEach(() => {
    // Save original AUTH_PROVIDER
    originalAuthProvider = process.env['AUTH_PROVIDER']

    // Reset mock functions
    vi.clearAllMocks()
  })

  afterEach(() => {
    // Restore original AUTH_PROVIDER
    if (originalAuthProvider === undefined) {
      process.env['AUTH_PROVIDER'] = undefined
    } else {
      process.env['AUTH_PROVIDER'] = originalAuthProvider
    }
  })

  describe('getCookieValue', () => {
    it('should extract cookie value from header', () => {
      const cookieHeader = 'moatless_session=abc123; other=value'
      const result = getCookieValue(cookieHeader, 'moatless_session')
      expect(result).toBe('abc123')
    })

    it('should return null if cookie not found', () => {
      const cookieHeader = 'other=value'
      const result = getCookieValue(cookieHeader, 'moatless_session')
      expect(result).toBeNull()
    })

    it('should return null if header is undefined', () => {
      const result = getCookieValue(undefined, 'moatless_session')
      expect(result).toBeNull()
    })

    it('should handle cookies with spaces', () => {
      const cookieHeader = 'moatless_session=abc123; other = value ; third=test'
      const result = getCookieValue(cookieHeader, 'moatless_session')
      expect(result).toBe('abc123')
    })

    it('should handle empty cookie value', () => {
      const cookieHeader = 'moatless_session=; other=value'
      const result = getCookieValue(cookieHeader, 'moatless_session')
      expect(result).toBeNull()
    })
  })

  describe('validateProxySession', () => {
    it('should return undefined when authentication is disabled', async () => {
      process.env['AUTH_PROVIDER'] = 'disabled'

      const mockReq = {
        headers: {},
      } as IncomingMessage

      const result = await validateProxySession(mockReq, mockLogger)
      expect(result).toBeUndefined()
    })

    it('should return undefined when AUTH_PROVIDER is not set', async () => {
      process.env['AUTH_PROVIDER'] = undefined

      const mockReq = {
        headers: {},
      } as IncomingMessage

      const result = await validateProxySession(mockReq, mockLogger)
      expect(result).toBeUndefined()
    })

    it('should return null when no session cookie is present', async () => {
      process.env['AUTH_PROVIDER'] = 'auth0'

      const mockReq = {
        headers: {},
      } as IncomingMessage

      const result = await validateProxySession(mockReq, mockLogger)
      expect(result).toBeNull()
    })

    it('should return null when session is invalid', async () => {
      process.env['AUTH_PROVIDER'] = 'auth0'

      const mockReq = {
        headers: {
          cookie: 'moatless_session=invalid-session',
        },
      } as IncomingMessage

      mockAuthService.getSession.mockResolvedValue(null)

      const result = await validateProxySession(mockReq, mockLogger)
      expect(result).toBeNull()
    })

    it('should return session object when session is valid', async () => {
      process.env['AUTH_PROVIDER'] = 'auth0'

      const mockSession: UserSession = {
        id: 'session-123',
        provider: 'auth0',
        userId: 'user-123',
        userLogin: 'testuser',
        userEmail: 'test@example.com',
        userName: 'Test User',
        avatarUrl: null,
        role: 'user',
        createdAt: new Date(),
        expiresAt: new Date(Date.now() + 3600000),
      }

      const mockReq = {
        headers: {
          cookie: 'moatless_session=valid-session',
        },
      } as IncomingMessage

      mockAuthService.getSession.mockResolvedValue(mockSession)

      const result = await validateProxySession(mockReq, mockLogger)
      expect(result).toEqual(mockSession)
      expect(mockAuthService.getSession).toHaveBeenCalledWith('valid-session')
    })

    it('should return null when AuthService throws error', async () => {
      process.env['AUTH_PROVIDER'] = 'auth0'

      const mockReq = {
        headers: {
          cookie: 'moatless_session=error-session',
        },
      } as IncomingMessage

      mockAuthService.getSession.mockRejectedValue(new Error('Database error'))

      const result = await validateProxySession(mockReq, mockLogger)
      expect(result).toBeNull()
    })
  })

  describe('isProxyRequestAuthorized', () => {
    it('should return true when authentication is disabled', async () => {
      process.env['AUTH_PROVIDER'] = 'disabled'

      const mockReq = {
        headers: {},
      } as IncomingMessage

      const result = await isProxyRequestAuthorized(mockReq, mockLogger)
      expect(result).toBe(true)
    })

    it('should return true when AUTH_PROVIDER is not set', async () => {
      process.env['AUTH_PROVIDER'] = undefined

      const mockReq = {
        headers: {},
      } as IncomingMessage

      const result = await isProxyRequestAuthorized(mockReq, mockLogger)
      expect(result).toBe(true)
    })

    it('should return false when no session cookie is present and auth is enabled', async () => {
      process.env['AUTH_PROVIDER'] = 'auth0'

      const mockReq = {
        headers: {},
      } as IncomingMessage

      const result = await isProxyRequestAuthorized(mockReq, mockLogger)
      expect(result).toBe(false)
    })

    it('should return false when session is invalid and auth is enabled', async () => {
      process.env['AUTH_PROVIDER'] = 'auth0'

      const mockReq = {
        headers: {
          cookie: 'moatless_session=invalid-session',
        },
      } as IncomingMessage

      mockAuthService.getSession.mockResolvedValue(null)

      const result = await isProxyRequestAuthorized(mockReq, mockLogger)
      expect(result).toBe(false)
    })

    it('should return true when session is valid and auth is enabled', async () => {
      process.env['AUTH_PROVIDER'] = 'auth0'

      const mockSession: UserSession = {
        id: 'session-123',
        provider: 'auth0',
        userId: 'user-123',
        userLogin: 'testuser',
        userEmail: 'test@example.com',
        userName: 'Test User',
        avatarUrl: null,
        role: 'user',
        createdAt: new Date(),
        expiresAt: new Date(Date.now() + 3600000),
      }

      const mockReq = {
        headers: {
          cookie: 'moatless_session=valid-session',
        },
      } as IncomingMessage

      mockAuthService.getSession.mockResolvedValue(mockSession)

      const result = await isProxyRequestAuthorized(mockReq, mockLogger)
      expect(result).toBe(true)
    })
  })
})
