import type { User, UserSession } from '@moatless/types'

/**
 * Create a synthetic session for internal operations (e.g., email inbound, webhooks).
 * These sessions are short-lived and used to establish user context for operations
 * that bypass normal authentication middleware.
 *
 * @param user - The user to create a session for
 * @param provider - The provider identifier (e.g., 'email', 'webhook', 'internal')
 * @returns A UserSession object suitable for use with SessionContext.run()
 */
export function createSyntheticSession(user: User, provider = 'internal'): UserSession {
  return {
    id: `${provider}-${user.id}-${Date.now()}`,
    provider,
    userId: user.id,
    userLogin: user.login,
    userEmail: user.email,
    userName: user.name,
    avatarUrl: user.avatarUrl,
    role: user.role,
    createdAt: new Date(),
    expiresAt: new Date(Date.now() + 60 * 1000), // 1 minute expiry
  }
}
