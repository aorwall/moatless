import { AsyncLocalStorage } from 'node:async_hooks'
import type { UserSession } from '@moatless/types'

/**
 * AsyncLocalStorage instance for storing session per request
 */
const sessionStorage = new AsyncLocalStorage<UserSession | undefined>()

/**
 * Run a function with the given session context
 */
function runWithSession<T>(session: UserSession | undefined, fn: () => T): T {
  return sessionStorage.run(session, fn)
}

/**
 * Get the current session from the request context
 * Returns undefined if no session is set or if called outside request context
 */
function getCurrentSession(): UserSession | undefined {
  return sessionStorage.getStore()
}

/**
 * Get the current user ID from the session
 * Returns undefined if no session is set
 */
function getCurrentUserId(): number | undefined {
  return getCurrentSession()?.userId
}

/**
 * Get the current group ID from the session
 * Returns undefined if no session is set or no group is assigned
 */
function getCurrentGroupId(): string | undefined {
  return getCurrentSession()?.groupId
}

/**
 * Check if there is an active session
 */
function hasSession(): boolean {
  return getCurrentSession() !== undefined
}

/**
 * Get the current access token from the session
 * Returns undefined if no session is set
 */
function getAccessToken(): string | undefined {
  return getCurrentSession()?.accessToken
}

/**
 * SessionContext provides access to the current user session
 * Uses AsyncLocalStorage to maintain session per request without explicit passing
 */
export const SessionContext = {
  run: runWithSession,
  getCurrentSession,
  getCurrentUserId,
  getCurrentGroupId,
  hasSession,
  getAccessToken,
}
