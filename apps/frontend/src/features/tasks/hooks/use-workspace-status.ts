import type { Task, WorkspaceStatusResponse } from '@moatless/types'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
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
 */
export function useWorkspaceRetry(taskId: string) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: () => tasksApi.retryWorkspace(taskId),
    onSuccess: () => {
      // Invalidate workspace status to trigger re-fetch
      // Status will transition: error → initializing → ready/error
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.workspaceStatus(taskId) })
    },
  })
}
