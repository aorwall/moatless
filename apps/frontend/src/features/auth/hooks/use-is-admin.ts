import { useAuth } from './use-auth'

/**
 * Convenience hook to check if the current user is an admin.
 * @returns true if user is authenticated and has admin role
 */
export function useIsAdmin(): boolean {
  const { isAdmin } = useAuth()
  return isAdmin
}
