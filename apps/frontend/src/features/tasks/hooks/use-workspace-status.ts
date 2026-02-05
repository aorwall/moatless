import type { Task, WorkspaceStatusResponse } from '@moatless/types'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useMutation } from '@tanstack/react-query'
import { tasksApi } from '../api/tasks'
import { useTaskWorkspaceStatusQuery } from './use-task-queries'

interface UseWorkspaceStatusOptions
  extends Omit<UseQueryOptions<WorkspaceStatusResponse>, 'queryKey' | 'queryFn'> {
  enabled?: boolean
}

/**
 * Hook to fetch workspace status for a task
 * - Fetches for active tasks (open, running, error)
 * - Stops fetching for terminal states (closed, merged)
 * - Polls intelligently based on environment/agent state
 * - Provides derived states for components
 */
export function useWorkspaceStatus(
  taskId: string,
  task?: Task,
  options?: UseWorkspaceStatusOptions,
) {
  const isEnabled = options?.enabled ?? true

  return useTaskWorkspaceStatusQuery(taskId, {
    ...options,
    enabled: isEnabled && task?.status !== 'closed' && task?.status !== 'merged',
  } as Parameters<typeof useTaskWorkspaceStatusQuery>[1])
}

/**
 * Hook to retry workspace provisioning after a failure
 * Clears error state and triggers re-provisioning
 * Note: Status updates are handled by SSE events in global-event-stream-provider
 */
export function useWorkspaceRetry(taskId: string) {
  return useMutation({
    mutationFn: () => tasksApi.retryWorkspace(taskId),
  })
}

/**
 * Hook to restart workspace by stopping the current sandbox and creating a new one
 * Preserves workspace data (PVC in K8s)
 * Note: Status updates are handled by SSE events in global-event-stream-provider
 */
export function useWorkspaceRestart(taskId: string) {
  return useMutation({
    mutationFn: () => tasksApi.restartWorkspace(taskId),
  })
}
