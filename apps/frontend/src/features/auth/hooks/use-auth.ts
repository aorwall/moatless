import { useQuery } from '@tanstack/react-query'
import { ApiError } from '@/lib/api/client'
import { getCurrentUser } from '../api/auth.api'

// API base URL for auth redirects
const API_BASE_URL = import.meta.env.PROD
  ? ''
  : import.meta.env.VITE_API_URL || 'http://localhost:8000'

/**
 * Hook to get the current authenticated user
 * Redirects to /auth/login on 401 response from /auth/me
 */
export function useAuth() {
  const { data: session, isLoading } = useQuery({
    queryKey: ['auth', 'session'],
    queryFn: getCurrentUser,
    retry: false, // Don't retry on 401
    staleTime: 5 * 60 * 1000, // Consider fresh for 5 minutes
    throwOnError: (error) => {
      // Redirect to login on 401 from /auth/me (backend determines provider)
      if (error instanceof ApiError && error.status === 401) {
        if (!window.location.pathname.startsWith('/auth')) {
          window.location.href = `${API_BASE_URL}/api/v1/auth/login`
        }
        return false // Don't throw the error
      }
      return true // Throw other errors
    },
  })

  /**
   * Logout by navigating to the logout endpoint
   * Uses GET request via window.location to allow proper redirect handling
   * For Auth0 users, backend will redirect to Auth0 logout, then back to app
   * For GitHub users, backend will return JSON and redirect to home
   */
  const logout = () => {
    window.location.href = `${API_BASE_URL}/api/v1/auth/logout`
  }

  return {
    user: session?.user ?? null,
    role: session?.user?.role ?? null,
    isAdmin: session?.user?.role === 'admin',
    expiresAt: session?.expiresAt ?? null,
    isAuthenticated: !!session,
    isLoading,
    logout,
  }
}
