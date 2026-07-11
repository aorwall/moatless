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
 * Handle 401 errors globally by redirecting to login.
 * Errors from the generated API client have a `status` property added via Object.assign,
 * so we check for that instead of instanceof.
 */
function handleAuthError(error: Error) {
  if ('status' in error && (error as { status: number }).status === 401) {
    redirectToLogin()
  }
}

/**
 * Mark the backend unreachable when a request fails with a 503 / network
 * error, surfacing the root-level `ServerDownGate`. A successful request
 * clears the flag (see the `QueryCache` `onSuccess` below).
 */
function handleServerStatusError(error: Error) {
  if (isServerUnavailable(error)) {
    serverStatusStore.setUnavailable(true)
  }
}

/**
 * Retry only transient server-side failures (500/502/504), at most twice.
 * 4xx responses are non-retriable client errors, and 503/network failures
 * are owned by the `ServerDownGate` flow via `serverStatusStore` — retrying
 * those would only delay the "server unavailable" screen.
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
    onError: (error) => {
      handleAuthError(error)
      handleServerStatusError(error)
    },
    onSuccess: () => {
      // Any successful request means the server is reachable again.
      serverStatusStore.setUnavailable(false)
    },
  }),
  // The MutationCache callbacks run for EVERY mutation, unlike
  // `defaultOptions.mutations.onError` which is replaced when a caller
  // passes its own `onError` — so auth/availability handling lives here.
  mutationCache: new MutationCache({
    onError: (error, _variables, _context, mutation) => {
      handleAuthError(error)
      handleServerStatusError(error)
      // Fallback toast for fire-and-forget mutations that surface no error UX
      // of their own. 401 and 503/network already have dedicated handling
      // (login redirect / ServerDownGate), so skip those.
      const status = (error as { status?: unknown }).status
      if (!mutation.options.onError && status !== 401 && !isServerUnavailable(error)) {
        toast.error(getErrorMessage(error))
      }
    },
    onSuccess: () => {
      serverStatusStore.setUnavailable(false)
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
