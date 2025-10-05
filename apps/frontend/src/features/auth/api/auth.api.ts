import { buildPath, type RouteSuccess } from '@moatless/api-contract'
import { getCurrentUserRoute, logoutRoute } from '@moatless/api-contract/routes/auth'
import { apiClient } from '@/lib/api/client'

type SessionResponse = RouteSuccess<typeof getCurrentUserRoute>

/**
 * Get the current authenticated user session
 */
export async function getCurrentUser(): Promise<SessionResponse> {
  const path = buildPath(getCurrentUserRoute)
  return apiClient.get<SessionResponse>(path)
}

/**
 * Logout and destroy the current session
 */
export async function logout(): Promise<RouteSuccess<typeof logoutRoute>> {
  const path = buildPath(logoutRoute)
  return await apiClient.post<RouteSuccess<typeof logoutRoute>>(path)
}
