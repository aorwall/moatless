import type { WorkspaceStatusResponse } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

/**
 * Client for the sandbox-controller API.
 *
 * Routes through the main domain at /api/sandbox/... — ALB forwards
 * to the Rust sandbox-controller service. Auth is handled by the
 * same cookie-based session as the main app.
 */
export const sandboxApi = {
  /** Get full workspace status (pod status + servers + proxy URLs). */
  getWorkspaceStatus(taskId: string): Promise<WorkspaceStatusResponse> {
    return apiClient.get<WorkspaceStatusResponse>(`/api/sandbox/v1/tasks/${taskId}/status`)
  },

  /** Restart workspace (delete pods, Deployment recreates them). */
  restartWorkspace(taskId: string): Promise<void> {
    return apiClient.post<void>(`/api/sandbox/v1/tasks/${taskId}/restart`)
  },

  /** Stop workspace (scale Deployment to 0). */
  stopWorkspace(taskId: string): Promise<void> {
    return apiClient.post<void>(`/api/sandbox/v1/tasks/${taskId}/stop`)
  },

  /** Start a stopped workspace (scale Deployment to 1). */
  startWorkspace(taskId: string): Promise<void> {
    return apiClient.post<void>(`/api/sandbox/v1/tasks/${taskId}/start`)
  },
}
