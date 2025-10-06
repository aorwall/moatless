import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ApiError } from '@/lib/api/client'
import { getCurrentUser, logout as logoutApi } from '../api/auth.api'

// API base URL for auth redirects
const API_BASE_URL = import.meta.env.PROD
  ? ''
  : import.meta.env.VITE_API_URL || 'http://localhost:8000'

/**
 * Hook to get the current authenticated user
 * Redirects to /auth/login on 401 response from /auth/me
 */
export function useAuth() {
  const queryClient = useQueryClient()

  const { data: session, isLoading } = useQuery({
    queryKey: ['auth', 'session'],
    queryFn: getCurrentUser,
    retry: false, // Don't retry on 401
    staleTime: 5 * 60 * 1000, // Consider fresh for 5 minutes
    throwOnError: (error) => {
      // Redirect to GitHub OAuth initiation on 401 from /auth/me
      if (error instanceof ApiError && error.status === 401) {
        if (!window.location.pathname.startsWith('/auth')) {
          window.location.href = `${API_BASE_URL}/api/v1/auth/github`
        }
        return false // Don't throw the error
      }
      return true // Throw other errors
    },
  })

  const logoutMutation = useMutation({
    mutationFn: logoutApi,
    onSuccess: () => {
      // Clear all queries after logout
      queryClient.clear()
      // Reload to reset app state
      window.location.href = '/'
    },
  })

  return {
    user: session?.user ?? null,
    expiresAt: session?.expiresAt ?? null,
    isAuthenticated: !!session,
    isLoading,
    logout: () => logoutMutation.mutate(),
  }
}
