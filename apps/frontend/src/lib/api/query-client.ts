import { QueryCache, QueryClient } from '@tanstack/react-query'

// API base URL for auth redirects
const API_BASE_URL = import.meta.env.PROD ? '' : (import.meta.env.VITE_API_URL ?? '')

const AUTH_REDIRECT_KEY = 'auth_redirect_ts'
const AUTH_REDIRECT_COOLDOWN_MS = 30_000

/**
 * Redirect to the login endpoint with a cooldown guard to prevent infinite loops.
 * If we already redirected within the last 30 seconds, skip to avoid a cycle
 * (e.g., when the login endpoint itself fails or cookies don't persist).
 */
export function redirectToLogin() {
  if (window.location.pathname.startsWith('/auth')) {
    return
  }

  const last = sessionStorage.getItem(AUTH_REDIRECT_KEY)
  if (last && Date.now() - Number(last) < AUTH_REDIRECT_COOLDOWN_MS) {
    return
  }

  sessionStorage.setItem(AUTH_REDIRECT_KEY, String(Date.now()))
  window.location.href = `${API_BASE_URL}/api/v1/auth/login`
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

export const queryClient = new QueryClient({
  queryCache: new QueryCache({
    onError: handleAuthError,
  }),
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60 * 5, // 5 minutes
      gcTime: 1000 * 60 * 10, // 10 minutes
      retry: false,
      refetchOnWindowFocus: false,
    },
    mutations: {
      retry: false,
      onError: handleAuthError,
    },
  },
})
