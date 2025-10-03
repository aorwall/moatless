import type { SessionResponse } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

/**
 * Get the current authenticated user session
 */
export async function getCurrentUser(): Promise<SessionResponse> {
  return apiClient.get<SessionResponse>('/auth/me')
}

/**
 * Logout and destroy the current session
 */
export async function logout(): Promise<void> {
  await apiClient.post<void>('/auth/logout')
}
