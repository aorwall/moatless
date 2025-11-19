import { QueryCache, QueryClient } from '@tanstack/react-query'
import { ApiError } from './client'

// API base URL for auth redirects
const API_BASE_URL = import.meta.env.PROD
  ? ''
  : import.meta.env.VITE_API_URL || 'http://localhost:8000'

/**
 * Handle 401 errors globally by redirecting to login
 */
function handleAuthError(error: Error) {
  // Handle 401 errors globally - redirect to login
  // Avoid redirect loops - don't redirect if already on auth pages
  if (
    error instanceof ApiError &&
    error.status === 401 &&
    !window.location.pathname.startsWith('/auth')
  ) {
    window.location.href = `${API_BASE_URL}/api/v1/auth/login`
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
