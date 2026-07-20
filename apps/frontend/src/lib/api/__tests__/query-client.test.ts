import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { queryClient } from '../query-client'

/**
 * These tests lock in the `meta: { skipAuthRedirect: true }` opt-out.
 *
 * `handleAuthError` is wired into both `QueryCache.onError` and
 * `MutationCache.onError`, so without the opt-out a wrong-password 401 from the
 * login form would navigate away from `/login`. Worse, even when the pathname
 * guard stops the navigation, reaching the redirect still stamps
 * `auth_redirect_ts` and burns the 30-second cooldown — so a genuine session
 * expiry moments later would silently fail to redirect.
 */
describe('queryClient auth-redirect opt-out', () => {
  const AUTH_REDIRECT_KEY = 'auth_redirect_ts'

  function unauthorized(): Error {
    return Object.assign(new Error('Unauthorized'), { status: 401 })
  }

  const realLocation = window.location

  /**
   * jsdom cannot navigate, and its `location.href` setter is non-configurable
   * so it cannot be spied. Swap in a plain stand-in: the pathname puts the test
   * on a non-login path (so the incidental pathname guard in `redirectToLogin`
   * cannot mask a missing opt-out), and the `href` write is a no-op. The
   * assertion is on the cooldown stamp, which is what actually breaks the next
   * genuine redirect.
   */
  beforeEach(() => {
    sessionStorage.clear()
    queryClient.clear()
    Object.defineProperty(window, 'location', {
      configurable: true,
      writable: true,
      value: { pathname: '/tasks', href: '' },
    })
  })

  afterEach(() => {
    Object.defineProperty(window, 'location', {
      configurable: true,
      writable: true,
      value: realLocation,
    })
    sessionStorage.clear()
    queryClient.clear()
  })

  it('does not redirect or burn the cooldown when a query opts out', async () => {
    await queryClient
      .fetchQuery({
        queryKey: ['auth', 'password-login'],
        queryFn: () => Promise.reject(unauthorized()),
        retry: false,
        meta: { skipAuthRedirect: true },
      })
      .catch(() => {
        /* expected rejection */
      })

    expect(sessionStorage.getItem(AUTH_REDIRECT_KEY)).toBeNull()
  })

  it('does not redirect or burn the cooldown when a mutation opts out', async () => {
    await queryClient
      .getMutationCache()
      .build(
        queryClient,
        {
          mutationFn: () => Promise.reject(unauthorized()),
          meta: { skipAuthRedirect: true },
        },
        undefined,
      )
      .execute(undefined)
      .catch(() => {
        /* expected rejection */
      })

    expect(sessionStorage.getItem(AUTH_REDIRECT_KEY)).toBeNull()
  })

  it('still redirects on a 401 from a query without the opt-out', async () => {
    await queryClient
      .fetchQuery({
        queryKey: ['tasks', 'list'],
        queryFn: () => Promise.reject(unauthorized()),
        retry: false,
      })
      .catch(() => {
        /* expected rejection */
      })

    expect(sessionStorage.getItem(AUTH_REDIRECT_KEY)).not.toBeNull()
  })

  it('still redirects on a 401 from a mutation without the opt-out', async () => {
    await queryClient
      .getMutationCache()
      .build(queryClient, { mutationFn: () => Promise.reject(unauthorized()) }, undefined)
      .execute(undefined)
      .catch(() => {
        /* expected rejection */
      })

    expect(sessionStorage.getItem(AUTH_REDIRECT_KEY)).not.toBeNull()
  })
})
