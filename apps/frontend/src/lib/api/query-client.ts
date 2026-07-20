import { MutationCache, QueryCache, QueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

import { getErrorMessage } from '@/lib/utils/error'
import { isServerUnavailable, serverStatusStore } from './server-status'

const AUTH_REDIRECT_KEY = 'auth_redirect_ts'
const AUTH_REDIRECT_COOLDOWN_MS = 30_000

/**
 * Redirect to the login endpoint with a cooldown guard to prevent infinite loops.
 * If we already redirected within the last 30 seconds, skip to avoid a cycle
 * (e.g., when the login endpoint itself fails or cookies don't persist).
 */
export function redirectToLogin() {
  if (window.location.pathname === '/login' || window.location.pathname.startsWith('/auth')) {
    return
  }

  const last = sessionStorage.getItem(AUTH_REDIRECT_KEY)
  if (last && Date.now() - Number(last) < AUTH_REDIRECT_COOLDOWN_MS) {
    return
  }

  sessionStorage.setItem(AUTH_REDIRECT_KEY, String(Date.now()))
  window.location.href = '/login'
}

/**
 * Opt a query or mutation out of the global 401 → `/login` redirect via
 * `meta: { skipAuthRedirect: true }`.
 *
 * Required for the password login request itself: a wrong password answers
 * 401, and without this the redirect stamps `auth_redirect_ts` and burns the
 * 30-second cooldown — so a genuine session expiry moments later would
 * silently fail to redirect.
 */
function skipsAuthRedirect(meta: Record<string, unknown> | undefined): boolean {
  return meta?.['skipAuthRedirect'] === true
}

/**
 * Handle 401 errors globally by redirecting to login.
 * Errors from the generated API client have a `status` property added via Object.assign,
 * so we check for that instead of instanceof.
 */
function handleAuthError(error: Error, meta: Record<string, unknown> | undefined) {
  if (skipsAuthRedirect(meta)) {
    return
  }
  if ('status' in error && (error as { status: number }).status === 401) {
    redirectToLogin()
  }
}

/**
 * True only for the auth-session probe (`GET /api/v1/auth/me`, tagged via
 * `meta: { serverHealthProbe: true }`). That single request is the source of
 * truth for backend reachability — a 503/network failure from any other
 * endpoint (e.g. a sandbox files/tree fetch) must fail only its own part of
 * the UI, not surface the full-page `ServerDownGate`.
 */
function isServerHealthProbe(query: { meta?: Record<string, unknown> | undefined }): boolean {
  return query.meta?.['serverHealthProbe'] === true
}

/**
 * Retry only transient server-side failures (500/502/504), at most twice.
 * 4xx responses are non-retriable client errors, and 503/network failures
 * are treated as "server unavailable" — retrying those would only delay the
 * error surfacing (the auth probe drives the `ServerDownGate`; other queries
 * surface their own local error UI).
 */
function retryTransientErrors(failureCount: number, error: Error): boolean {
  if (failureCount >= 2 || isServerUnavailable(error)) {
    return false
  }
  const status = (error as { status?: unknown }).status
  return typeof status === 'number' && status >= 500
}

export const queryClient = new QueryClient({
  queryCache: new QueryCache({
    onError: (error, query) => {
      handleAuthError(error, query.meta)
      // Only the auth-session probe flips the global "server unavailable"
      // screen. A 503 from any other endpoint fails just its own part of the
      // UI (that query's own error state), not the whole app.
      if (isServerHealthProbe(query) && isServerUnavailable(error)) {
        serverStatusStore.setUnavailable(true)
      }
    },
    onSuccess: (_data, query) => {
      // A successful probe means the server is reachable again. Scoped to the
      // probe so the flag's lifecycle stays owned by a single request.
      if (isServerHealthProbe(query)) {
        serverStatusStore.setUnavailable(false)
      }
    },
  }),
  // The MutationCache callbacks run for EVERY mutation, unlike
  // `defaultOptions.mutations.onError` which is replaced when a caller
  // passes its own `onError` — so auth handling lives here.
  mutationCache: new MutationCache({
    onError: (error, _variables, _context, mutation) => {
      handleAuthError(error, mutation.meta)
      // Fallback toast for fire-and-forget mutations that surface no error UX
      // of their own. 401 already has dedicated handling (login redirect), so
      // skip it; a 503 here fails just this mutation, so let it toast.
      const status = (error as { status?: unknown }).status
      if (!mutation.options.onError && status !== 401) {
        toast.error(getErrorMessage(error))
      }
    },
  }),
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60 * 5, // 5 minutes
      gcTime: 1000 * 60 * 10, // 10 minutes
      retry: retryTransientErrors,
      refetchOnWindowFocus: false,
    },
    mutations: {
      retry: false,
    },
  },
})
